//! Read-only object handles replace the Windows pointers passed to layeredwindow.
//! The submitted DIB rows form a window snapshot until the next submission.
use super::*;

struct WindowHandle(usize);
struct BufferHandle(usize);

impl WindowHandle {
    fn read(services: &Services, value: &Value) -> Result<Self> {
        let id = object(value)?.context("layeredwindow requires a Window handle")?;
        ensure!(
            services
                .windows
                .get(&id)
                .is_some_and(|w| w.constructed && !w.invalidating),
            "layeredwindow requires a live Window handle"
        );
        Ok(Self(id))
    }
}

impl BufferHandle {
    fn read(services: &Services, value: &Value) -> Result<Self> {
        let id = object(value)?.context("layeredwindow requires a Layer buffer handle")?;
        let layer = services
            .layers
            .get(&id)
            .filter(|layer| layer.constructed)
            .context("layeredwindow requires a live Layer buffer handle")?;
        layer.bitmap()?;
        Ok(Self(id))
    }
}

impl Services {
    pub(crate) fn window_handle(&self, context: &Value) -> Result<Value> {
        Ok(bound(WindowHandle::read(self, context)?.0))
    }

    pub(super) fn layer_buffer(&self, id: usize) -> Result<Value> {
        let layer = &self.layers[&id];
        Ok(if layer.image.is_some() {
            bound(id)
        } else {
            Value::NULL
        })
    }

    pub(crate) fn layered_window(&mut self, args: &[Value], budget: &mut u64) -> Result<Value> {
        ensure!(args.len() >= 5, "layeredwindow requires five arguments");
        let window = WindowHandle::read(self, &args[0])?;
        let buffer = BufferHandle::read(self, &args[1])?;
        let image = self.layers[&buffer.0].image.as_ref().unwrap();
        let pitch = args[2].integer()?;
        ensure!(
            pitch == -i64::from(image.width) * 4,
            "layeredwindow requires the Layer buffer pitch"
        );
        ensure!(
            args[3].integer()? == i64::from(image.width)
                && args[4].integer()? == i64::from(image.height),
            "layeredwindow dimensions do not match the Layer buffer"
        );
        // Retained snapshots have their own bound, including when the source
        // layer detaches its shared bitmap or is invalidated after submission.
        let retained: usize = self
            .windows
            .iter()
            .filter(|(id, _)| **id != window.0)
            .filter_map(|(_, w)| w.layered_image.as_ref())
            .map(|image| image.rgba.len())
            .sum();
        ensure!(
            retained + image.rgba.len() <= 64 << 20,
            "layered window snapshots exceed memory limit"
        );
        *budget = budget
            .checked_sub((image.rgba.len() / 4) as u64)
            .ok_or_else(|| unsupported("layeredwindow execution budget exceeded"))?;
        self.windows.get_mut(&window.0).unwrap().layered_image = Some(image.clone());
        Ok(Value::Void)
    }
}
