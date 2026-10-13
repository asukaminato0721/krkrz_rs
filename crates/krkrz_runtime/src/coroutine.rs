//! Cooperative native tasks driven by the host clock and explicit signals.
//! async-executor runs ready tasks. Commands retain task creation order.
//! Tasks must return control promptly. Script callbacks still use EventQueue.

use anyhow::{Result, ensure};
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TaskId(u64);

struct Registration {
    ready: Rc<Cell<bool>>,
    waker: Waker,
}

#[derive(Default)]
struct Waits {
    now: u64,
    step: u64,
    deadlines: BTreeMap<(u64, usize), Registration>,
    steps: BTreeMap<(u64, usize), Registration>,
    events: BTreeMap<u64, BTreeMap<usize, Registration>>,
}

/// A task can wait without borrowing Session or Services across a suspension.
/// Clock values use the same units that the caller passes to advance.
#[derive(Clone)]
pub struct TaskContext {
    waits: Rc<RefCell<Waits>>,
}

impl TaskContext {
    pub fn now(&self) -> u64 {
        self.waits.borrow().now
    }

    pub fn wait_until(&self, deadline: u64) -> Wait {
        self.wait(WaitKind::Deadline(deadline))
    }

    /// Yield until the next advance, including an advance at the same time.
    pub fn next_step(&self) -> Wait {
        self.wait(WaitKind::Step(self.waits.borrow().step.saturating_add(1)))
    }

    /// Wait for a signal. Signals before registration are not retained.
    pub fn wait_event(&self, event: u64) -> Wait {
        self.wait(WaitKind::Event(event))
    }

    fn wait(&self, kind: WaitKind) -> Wait {
        Wait {
            waits: self.waits.clone(),
            kind,
            ready: Rc::new(Cell::new(false)),
        }
    }
}

#[derive(Clone, Copy)]
enum WaitKind {
    Deadline(u64),
    Step(u64),
    Event(u64),
}

/// Awaiting this future yields control until its condition is met.
/// Dropping the future removes its registration, including on cancellation.
pub struct Wait {
    waits: Rc<RefCell<Waits>>,
    kind: WaitKind,
    ready: Rc<Cell<bool>>,
}

impl Wait {
    fn key(&self) -> usize {
        // The Rc keeps this address unique until the registration is removed.
        Rc::as_ptr(&self.ready) as usize
    }

    fn remove(&self, waits: &mut Waits) {
        let key = self.key();
        match self.kind {
            WaitKind::Deadline(time) => {
                waits.deadlines.remove(&(time, key));
            }
            WaitKind::Step(step) => {
                waits.steps.remove(&(step, key));
            }
            WaitKind::Event(event) => {
                if let Some(entries) = waits.events.get_mut(&event) {
                    entries.remove(&key);
                    if entries.is_empty() {
                        waits.events.remove(&event);
                    }
                }
            }
        }
    }
}

impl Future for Wait {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut waits = self.waits.borrow_mut();
        let ready = self.ready.get()
            || match self.kind {
                WaitKind::Deadline(time) => waits.now >= time,
                WaitKind::Step(step) => waits.step >= step,
                WaitKind::Event(_) => false,
            };
        if ready {
            self.remove(&mut waits);
            return Poll::Ready(());
        }
        let registration = Registration {
            ready: self.ready.clone(),
            waker: cx.waker().clone(),
        };
        let key = self.key();
        match self.kind {
            WaitKind::Deadline(time) => {
                waits.deadlines.insert((time, key), registration);
            }
            WaitKind::Step(step) => {
                waits.steps.insert((step, key), registration);
            }
            WaitKind::Event(event) => {
                waits
                    .events
                    .entry(event)
                    .or_default()
                    .insert(key, registration);
            }
        }
        Poll::Pending
    }
}

impl Drop for Wait {
    fn drop(&mut self) {
        self.remove(&mut self.waits.borrow_mut());
    }
}

type TaskFuture<T> = Pin<Box<dyn Future<Output = Option<T>>>>;
type FutureSlot<T> = Rc<RefCell<Option<TaskFuture<T>>>>;

struct Task<T> {
    // The slot permits immediate destruction of engine state on cancellation.
    future: FutureSlot<T>,
    handle: async_executor::Task<()>,
}

struct BatchTask<T> {
    id: TaskId,
    future: FutureSlot<T>,
    waits: Rc<RefCell<Waits>>,
    deferred: Rc<RefCell<BTreeMap<TaskId, Waker>>>,
    completed: Rc<RefCell<Vec<TaskId>>>,
    commands: Rc<RefCell<Vec<(TaskId, T)>>>,
    last_step: Option<u64>,
}

impl<T> Future for BatchTask<T> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let step = self.waits.borrow().step;
        if self.last_step == Some(step) {
            // A self-wake must not consume the entire host frame.
            self.deferred
                .borrow_mut()
                .insert(self.id, cx.waker().clone());
            return Poll::Pending;
        }
        self.last_step = Some(step);
        let mut slot = self.future.borrow_mut();
        let Some(future) = slot.as_mut() else {
            return Poll::Ready(());
        };
        match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(value) => {
                slot.take();
                if let Some(value) = value {
                    self.commands.borrow_mut().push((self.id, value));
                }
                self.completed.borrow_mut().push(self.id);
                Poll::Ready(())
            }
        }
    }
}

/// Send a native command without ending the coroutine.
/// Apply commands after advance returns, before advancing the next batch.
pub struct Emitter<T> {
    id: TaskId,
    commands: Rc<RefCell<Vec<(TaskId, T)>>>,
}

impl<T> Emitter<T> {
    pub fn emit(&self, command: T) {
        self.commands.borrow_mut().push((self.id, command));
    }
}

/// Host clock and dispatch adapter for async-executor::LocalExecutor.
/// Keep mutable engine access outside tasks. Apply commands in host dispatch phases.
pub struct CoroutineScheduler<T> {
    waits: Rc<RefCell<Waits>>,
    executor: async_executor::LocalExecutor<'static>,
    deferred: Rc<RefCell<BTreeMap<TaskId, Waker>>>,
    completed: Rc<RefCell<Vec<TaskId>>>,
    tasks: BTreeMap<TaskId, Task<T>>,
    commands: Rc<RefCell<Vec<(TaskId, T)>>>,
    next_id: u64,
}

impl<T> Default for CoroutineScheduler<T> {
    fn default() -> Self {
        Self {
            waits: Rc::new(RefCell::new(Waits::default())),
            executor: async_executor::LocalExecutor::new(),
            deferred: Rc::new(RefCell::new(BTreeMap::new())),
            completed: Rc::new(RefCell::new(Vec::new())),
            tasks: BTreeMap::new(),
            commands: Rc::new(RefCell::new(Vec::new())),
            next_id: 0,
        }
    }
}

impl<T: 'static> CoroutineScheduler<T> {
    /// Start the task on the next advance. The future can retain local state.
    pub fn spawn<F: Future<Output = T> + 'static>(
        &mut self,
        task: impl FnOnce(TaskContext) -> F,
    ) -> Result<TaskId> {
        self.spawn_inner(|cx, _| {
            let future = task(cx);
            async move { Some(future.await) }
        })
    }

    /// Run a coroutine that can emit commands across multiple waits.
    pub fn spawn_routine<F: Future<Output = ()> + 'static>(
        &mut self,
        task: impl FnOnce(TaskContext, Emitter<T>) -> F,
    ) -> Result<TaskId> {
        self.spawn_inner(|cx, emitter| {
            let future = task(cx, emitter);
            async move {
                future.await;
                None
            }
        })
    }

    fn spawn_inner<F: Future<Output = Option<T>> + 'static>(
        &mut self,
        task: impl FnOnce(TaskContext, Emitter<T>) -> F,
    ) -> Result<TaskId> {
        ensure!(self.tasks.len() < 100_000, "coroutine task limit exceeded");
        let id = TaskId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("coroutine task ID overflow"))?;
        let future: FutureSlot<T> = Rc::new(RefCell::new(Some(Box::pin(task(
            TaskContext {
                waits: self.waits.clone(),
            },
            Emitter {
                id,
                commands: self.commands.clone(),
            },
        )))));
        let handle = self.executor.spawn(BatchTask {
            id,
            future: future.clone(),
            waits: self.waits.clone(),
            deferred: self.deferred.clone(),
            completed: self.completed.clone(),
            commands: self.commands.clone(),
            last_step: None,
        });
        self.tasks.insert(id, Task { future, handle });
        Ok(id)
    }

    pub fn cancel(&mut self, id: TaskId) -> bool {
        let Some(task) = self.tasks.remove(&id) else {
            return false;
        };
        task.future.borrow_mut().take();
        self.deferred.borrow_mut().remove(&id);
        drop(task.handle);
        true
    }

    pub fn pending(&self) -> usize {
        self.tasks.len()
    }

    /// Return the earliest clock wait, for a host event loop's sleep deadline.
    pub fn next_deadline(&self) -> Option<u64> {
        self.waits
            .borrow()
            .deadlines
            .first_key_value()
            .map(|((time, _), _)| *time)
    }

    /// Wake all tasks currently registered for this event.
    pub fn notify_event(&mut self, event: u64) {
        let entries = self
            .waits
            .borrow_mut()
            .events
            .remove(&event)
            .unwrap_or_default();
        for registration in entries.into_values() {
            registration.ready.set(true);
            registration.waker.wake();
        }
    }

    /// Resume ready tasks. Each engine future runs at most once in this batch.
    pub fn advance(&mut self, now: u64) -> Result<Vec<(TaskId, T)>> {
        {
            let mut waits = self.waits.borrow_mut();
            ensure!(now >= waits.now, "coroutine clock cannot move backwards");
            let step = waits
                .step
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("coroutine step overflow"))?;
            waits.now = now;
            waits.step = step;
            wake_due(&mut waits.deadlines, now);
            wake_due(&mut waits.steps, step);
        }
        for waker in std::mem::take(&mut *self.deferred.borrow_mut()).into_values() {
            waker.wake();
        }
        while self.executor.try_tick() {}
        let finished = std::mem::take(&mut *self.completed.borrow_mut());
        for id in finished {
            self.tasks.remove(&id);
        }
        let mut completed = std::mem::take(&mut *self.commands.borrow_mut());
        completed.sort_by_key(|(id, _)| *id);
        Ok(completed)
    }
}

fn wake_due(entries: &mut BTreeMap<(u64, usize), Registration>, now: u64) {
    while entries
        .first_key_value()
        .is_some_and(|((time, _), _)| *time <= now)
    {
        let (_, registration) = entries.pop_first().unwrap();
        registration.ready.set(true);
        registration.waker.wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::poll_fn;

    #[test]
    fn local_state_survives_time_step_and_event_waits() {
        let mut scheduler = CoroutineScheduler::default();
        let trace = Rc::new(RefCell::new(Vec::new()));
        let log = trace.clone();
        let id = scheduler
            .spawn(move |cx| async move {
                let mut value = 40;
                log.borrow_mut().push(cx.now());
                cx.wait_until(10).await;
                value += 1;
                log.borrow_mut().push(cx.now());
                cx.next_step().await;
                log.borrow_mut().push(cx.now());
                cx.wait_event(7).await;
                value + 1
            })
            .unwrap();
        scheduler.notify_event(7); // A signal before registration is not retained.
        assert!(scheduler.advance(0).unwrap().is_empty());
        assert!(scheduler.advance(9).unwrap().is_empty());
        assert!(scheduler.advance(10).unwrap().is_empty());
        assert_eq!(*trace.borrow(), [0, 10]);
        assert!(scheduler.advance(10).unwrap().is_empty());
        assert_eq!(*trace.borrow(), [0, 10, 10]);
        scheduler.notify_event(8);
        assert!(scheduler.advance(11).unwrap().is_empty());
        scheduler.notify_event(7);
        assert_eq!(scheduler.advance(11).unwrap(), [(id, 42)]);
        assert_eq!(scheduler.pending(), 0);
    }

    #[test]
    fn sleeping_tasks_are_not_polled_on_each_advance() {
        let mut scheduler = CoroutineScheduler::default();
        let polls = Rc::new(Cell::new(0));
        let count = polls.clone();
        let id = scheduler
            .spawn(move |cx| {
                let mut wait = Box::pin(cx.wait_until(1000));
                poll_fn(move |cx| {
                    count.set(count.get() + 1);
                    wait.as_mut().poll(cx)
                })
            })
            .unwrap();
        for now in 0..1000 {
            assert!(scheduler.advance(now).unwrap().is_empty());
        }
        assert_eq!(polls.get(), 1);
        assert_eq!(scheduler.advance(1000).unwrap(), [(id, ())]);
        assert_eq!(polls.get(), 2);
        assert!(scheduler.waits.borrow().deadlines.is_empty());
    }

    #[test]
    fn simultaneous_wakes_follow_creation_order() {
        let mut scheduler = CoroutineScheduler::default();
        let first = scheduler
            .spawn(|cx| async move {
                cx.wait_event(1).await;
                "first"
            })
            .unwrap();
        let second = scheduler
            .spawn(|cx| async move {
                cx.wait_until(10).await;
                "second"
            })
            .unwrap();
        let third = scheduler
            .spawn(|cx| async move {
                cx.wait_event(2).await;
                "third"
            })
            .unwrap();
        scheduler.advance(0).unwrap();
        scheduler.notify_event(2);
        scheduler.notify_event(1);
        assert_eq!(
            scheduler.advance(10).unwrap(),
            [(first, "first"), (second, "second"), (third, "third"),]
        );
    }

    #[test]
    fn cancellation_removes_all_wait_kinds() {
        let mut scheduler = CoroutineScheduler::default();
        let time = scheduler
            .spawn(|cx| async move { cx.wait_until(10).await })
            .unwrap();
        let step = scheduler
            .spawn(|cx| async move { cx.next_step().await })
            .unwrap();
        let event = scheduler
            .spawn(|cx| async move { cx.wait_event(1).await })
            .unwrap();
        scheduler.advance(0).unwrap();
        let weak = Rc::downgrade(
            &scheduler
                .waits
                .borrow()
                .deadlines
                .first_key_value()
                .unwrap()
                .1
                .ready,
        );
        for id in [time, step, event] {
            assert!(scheduler.cancel(id));
            assert!(!scheduler.cancel(id));
        }
        assert!(weak.upgrade().is_none());
        let waits = scheduler.waits.borrow();
        assert!(waits.deadlines.is_empty());
        assert!(waits.steps.is_empty());
        assert!(waits.events.is_empty());
        drop(waits);
        scheduler.notify_event(1);
        assert!(scheduler.advance(10).unwrap().is_empty());
        assert_eq!(scheduler.pending(), 0);
    }

    #[test]
    fn self_wakes_yield_to_the_next_batch_and_stale_wakers_are_inert() {
        let mut scheduler = CoroutineScheduler::default();
        let polls = Rc::new(Cell::new(0));
        let count = polls.clone();
        let saved_waker = Rc::new(RefCell::new(None));
        let saved = saved_waker.clone();
        let id = scheduler
            .spawn(move |_| {
                poll_fn(move |cx| {
                    count.set(count.get() + 1);
                    *saved.borrow_mut() = Some(cx.waker().clone());
                    for _ in 0..10 {
                        cx.waker().wake_by_ref();
                    }
                    if count.get() == 3 {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
            })
            .unwrap();
        assert!(scheduler.advance(0).unwrap().is_empty());
        assert_eq!(polls.get(), 1);
        assert!(scheduler.advance(0).unwrap().is_empty());
        assert_eq!(scheduler.advance(0).unwrap(), [(id, ())]);
        saved_waker.borrow().as_ref().unwrap().wake_by_ref();
        assert!(scheduler.deferred.borrow().is_empty());
        assert!(scheduler.advance(0).unwrap().is_empty());
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn backward_time_does_not_consume_ready_tasks() {
        let mut scheduler = CoroutineScheduler::default();
        scheduler.advance(10).unwrap();
        let id = scheduler.spawn(|_| async { 42 }).unwrap();
        assert!(scheduler.advance(9).is_err());
        assert_eq!(scheduler.advance(10).unwrap(), [(id, 42)]);
    }

    #[test]
    fn routine_emits_across_waits_and_retains_its_final_command() {
        let mut scheduler = CoroutineScheduler::default();
        let id = scheduler
            .spawn_routine(|cx, out| async move {
                out.emit(1);
                cx.wait_until(10).await;
                out.emit(2);
                cx.next_step().await;
                out.emit(3);
            })
            .unwrap();
        assert_eq!(scheduler.advance(0).unwrap(), [(id, 1)]);
        assert_eq!(scheduler.next_deadline(), Some(10));
        assert!(scheduler.advance(9).unwrap().is_empty());
        assert_eq!(scheduler.advance(10).unwrap(), [(id, 2)]);
        assert_eq!(scheduler.next_deadline(), None);
        assert_eq!(scheduler.pending(), 1);
        assert_eq!(scheduler.advance(10).unwrap(), [(id, 3)]);
        assert_eq!(scheduler.pending(), 0);
    }

    #[test]
    fn cancel_and_scheduler_drop_release_captured_state_immediately() {
        let mut scheduler = CoroutineScheduler::<()>::default();
        let state = Rc::new(());
        let weak = Rc::downgrade(&state);
        let id = scheduler
            .spawn(move |cx| async move {
                cx.wait_event(7).await;
                drop(state);
            })
            .unwrap();
        assert!(scheduler.cancel(id)); // Cancel before the executor first runs.
        assert!(weak.upgrade().is_none());
        assert!(scheduler.advance(0).unwrap().is_empty());
        let state = Rc::new(());
        let weak = Rc::downgrade(&state);
        scheduler
            .spawn(move |cx| async move {
                cx.wait_event(7).await;
                drop(state);
            })
            .unwrap();
        scheduler.advance(0).unwrap();
        drop(scheduler);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn multiple_event_waits_in_one_task_have_separate_registrations() {
        let mut scheduler = CoroutineScheduler::default();
        let id = scheduler
            .spawn(|cx| {
                let mut first = Box::pin(cx.wait_event(1));
                let mut second = Box::pin(cx.wait_event(1));
                let mut first_ready = false;
                let mut second_ready = false;
                poll_fn(move |cx| {
                    first_ready |= !first_ready && first.as_mut().poll(cx).is_ready();
                    second_ready |= !second_ready && second.as_mut().poll(cx).is_ready();
                    if first_ready && second_ready {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
            })
            .unwrap();
        scheduler.advance(0).unwrap();
        assert_eq!(scheduler.waits.borrow().events[&1].len(), 2);
        scheduler.notify_event(1);
        assert_eq!(scheduler.advance(0).unwrap(), [(id, ())]);
        assert!(scheduler.waits.borrow().events.is_empty());
    }
}
