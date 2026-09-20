//! AlphaMovie registration, storage and transport controls. Observed against the
//! installed DLL with synthetic AJPM resources; see docs/alpha-movie-interface.json.
//! Pixel reconstruction uses the checked assets decoder. Queued movies and seeks
//! beyond the first frame remain explicit unsupported calls.
use crate::Services;
use anyhow::{Context, Result, ensure};
use krkrz_assets::amv::Movie;
use krkrz_tjs::{Value, Vm, unsupported};

pub(crate) struct Player {
    movie: Option<Movie>,
    playing: bool,
    looping: bool,
    next_loop: bool,
    preload: i32,
    packet: usize,
    displayed: u32,
    ready: bool,
    position: (i32,i32),
}
impl Default for Player {
    fn default() -> Self {
        Self {
            movie: None,
            playing: true,
            looping: true,
            next_loop: true,
            preload: 5,
            packet: 0,
            displayed: 0,
            ready: false,
            position: (0,0),
        }
    }
}
pub(crate) fn register(vm: &mut Vm) -> Result<()> {
    let class = vm.new_native_class("AlphaMovie", "AlphaMovie.@initialize")?;
    for method in [
        "AlphaMovie",
        "finalize",
        "open",
        "clear",
        "showNextImage",
        "isPlaying",
        "play",
        "stop",
        "setPosition",
        "setNextMovieFile",
    ] {
        vm.register_native_method(&class, method, &format!("AlphaMovie.{method}"))?;
    }
    for property in [
        "numOfFrame",
        "frame",
        "loop",
        "nextLoop",
        "preloadSamples",
        "screenWidth",
        "screenHeight",
        "FPSScale",
        "FPSRate",
    ] {
        let writable = matches!(property, "frame" | "loop" | "nextLoop" | "preloadSamples");
        vm.register_native_property(
            &class,
            property,
            Some(&format!("AlphaMovie.get:{property}")),
            writable
                .then(|| format!("AlphaMovie.set:{property}"))
                .as_deref(),
        )?;
    }
    vm.globals.insert("AlphaMovie".into(), class);
    Ok(())
}
impl Services {
    pub(crate) fn alpha_movie_call(
        &mut self,
        operation: &str,
        context: &Value,
        args: &[Value],
        budget: &mut u64,
    ) -> Result<Value> {
        let Value::Object(reference) = context else {
            anyhow::bail!("AlphaMovie requires an object context");
        };
        let id = reference
            .object
            .context("AlphaMovie requires a non-null context")?;
        if operation == "@initialize" {
            self.alpha_movies.entry(id).or_default();
            return Ok(Value::Void);
        }
        if operation == "@invalidate" {
            self.alpha_movies.remove(&id);
            return Ok(Value::Void);
        }
        ensure!(
            self.alpha_movies.contains_key(&id),
            "context has no AlphaMovie native instance"
        );
        if operation == "open" {
            let name = args
                .first()
                .context("AlphaMovie.open requires a storage name")?
                .text();
            let data = self.read_storage(&name)?;
            // A native parse cannot evade the session's execution budget. Every
            // packet needs at least 20 bytes; charge a conservative work bound.
            let charge = (data.len() / 20 + 1) as u64;
            *budget = budget
                .checked_sub(charge)
                .ok_or_else(|| unsupported("AlphaMovie open execution budget exceeded"))?;
            let movie = Movie::parse(data).with_context(|| format!("AlphaMovie.open({name})"))?;
            let player=self.alpha_movies.get_mut(&id).unwrap();
            player.movie = Some(movie);
            player.packet=0;
            player.displayed=0;
            player.ready=false;
            return Ok(Value::Void);
        }
        if operation == "showNextImage" {
            let target=args.first().context("AlphaMovie.showNextImage requires a Layer")?;
            let Value::Object(target)=target else { anyhow::bail!("AlphaMovie.showNextImage requires a Layer"); };
            let target=target.object.context("AlphaMovie.showNextImage requires a non-null Layer")?;
            ensure!(self.layers.contains_key(&target), "AlphaMovie.showNextImage requires a Layer");
            let player=self.alpha_movies.get_mut(&id).unwrap();
            if !player.playing || !player.ready { return Ok(Value::Integer(player.displayed as i64)); }
            let movie=player.movie.as_ref().context("AlphaMovie has no open movie")?;
            if player.packet >= movie.packets.len() {
                if !player.looping {return Ok(Value::Integer(player.displayed as i64));}
                player.packet=0;
            }
            let packet=&movie.packets[player.packet];
            let charge=packet.width as u64 * packet.height as u64 / 64 + 1;
            *budget=budget.checked_sub(charge).ok_or_else(||unsupported("AlphaMovie decode execution budget exceeded"))?;
            if let Some(image)=movie.decode_packet(player.packet)? {
                self.layers.get_mut(&target).unwrap().present_alpha_movie(&image,
                    player.position.0.saturating_add(packet.left as i32),
                    player.position.1.saturating_add(packet.top as i32))?;
            }
            player.displayed=packet.sequence;
            player.packet+=1;
            return Ok(Value::Integer(player.displayed as i64));
        }
        let player = self.alpha_movies.get_mut(&id).unwrap();
        let header = player.movie.as_ref().map(|movie| &movie.header);
        let integer = match operation {
            "AlphaMovie" | "finalize" => return Ok(Value::Void),
            "get:screenWidth" => header.map_or(0, |h| h.width as i64),
            "get:screenHeight" => header.map_or(0, |h| h.height as i64),
            "get:numOfFrame" => header.map_or(0, |h| h.frame_count as i64),
            "get:FPSRate" => header.map_or(0, |h| h.fps_rate as i64),
            "get:FPSScale" => header.map_or(0, |h| h.fps_scale as i64),
            "get:frame" => player.movie.as_ref().and_then(|m|m.packets.get(player.packet)).map_or(player.displayed,|p|p.sequence) as i64,
            "get:loop" => player.looping.into(),
            "get:nextLoop" => player.next_loop.into(),
            "get:preloadSamples" => player.preload.into(),
            "isPlaying" => player.playing.into(),
            "set:loop" | "set:nextLoop" | "set:preloadSamples" => {
                let n = args
                    .first()
                    .context("AlphaMovie property requires a value")?
                    .integer()? as i32;
                match operation {
                    "set:loop" => player.looping = n != 0,
                    "set:nextLoop" => player.next_loop = n != 0,
                    _ if (1..=30).contains(&n) => player.preload = n,
                    _ => {}
                }
                return Ok(Value::Void);
            }
            "play" => {
                ensure!(player.movie.is_some(), "AlphaMovie has no open movie");
                player.playing = true;
                player.ready = true;
                return Ok(Value::Void);
            }
            "stop" => {
                player.playing = false;
                return Ok(Value::Void);
            }
            "clear" => {
                // Metadata, transport and configuration are retained.
                player.ready = false;
                return Ok(Value::Void);
            }
            "set:frame" => {
                let frame = args
                    .first()
                    .context("AlphaMovie.frame requires a value")?
                    .integer()? as i32;
                if frame <= 0 || header.is_none_or(|h| frame as u32 >= h.frame_count) {
                    return Ok(Value::Void);
                }
                return Err(unsupported(
                    "AlphaMovie random frame seeking is not implemented",
                ));
            }
            "setPosition" => {
                ensure!(args.len()>=2,"AlphaMovie.setPosition requires two coordinates");
                player.position=(args[0].integer()? as i32,args[1].integer()? as i32);
                return Ok(Value::Void);
            }
            "setNextMovieFile" if args.is_empty() => {
                anyhow::bail!("AlphaMovie.setNextMovieFile requires a storage name")
            }
            _ => {
                return Err(unsupported(format!(
                    "AlphaMovie.{operation}: operation is not implemented"
                )));
            }
        };
        Ok(Value::Integer(integer))
    }
}

#[cfg(test)]
mod tests {
    use crate::Session;
    #[test]
    fn subclass_invalidation_releases_movie_storage() {
        let project = tempfile::tempdir().unwrap();
        let saves = tempfile::tempdir().unwrap();
        std::fs::write(
            project.path().join("empty.amv"),
            include_bytes!("../tests/fixtures/empty.amv"),
        )
        .unwrap();
        std::fs::write(project.path().join("startup.tjs"), "Plugins.link('AlphaMovie.dll');class A extends AlphaMovie {function A(){super.AlphaMovie();open(System.exePath+'empty.amv');}}var a=new A();invalidate a;").unwrap();
        let mut session = Session::open(project.path(), Some(saves.path()), false, 10_000).unwrap();
        session.startup().unwrap();
        assert!(session.services.alpha_movies.is_empty());
    }
}
