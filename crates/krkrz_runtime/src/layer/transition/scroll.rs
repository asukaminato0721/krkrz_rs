//! Pixel translation from Kirikiri's visual/TransIntf.cpp scroll provider.
use super::*;
use anyhow::bail;

enum From {
    Left,
    Top,
    Right,
    Bottom,
}
#[derive(Clone, Copy)]
enum Stay {
    Neither,
    Destination,
    Source,
}
#[derive(Clone, Copy)]
enum Edge {
    Low,
    High,
}

pub(super) struct Scroll {
    from: From,
    stay: Stay,
    extent: i32,
}
impl Scroll {
    pub(super) fn new(
        vm: &mut Vm,
        host: &mut Services,
        options: &Value,
        size: (i32, i32),
        budget: &mut u64,
    ) -> Result<Self> {
        let mut option = |name: &str| -> Result<i64> {
            let value =
                vm.get_property(options, &Value::string(name), true, false, host, budget)?;
            if matches!(value, Value::Void) {
                Ok(0)
            } else {
                value.integer()
            }
        };
        let from = match option("from")? {
            0 => From::Left,
            1 => From::Top,
            2 => From::Right,
            3 => From::Bottom,
            _ => bail!("scroll from must be sttLeft, sttTop, sttRight or sttBottom"),
        };
        let stay = match option("stay")? {
            0 => Stay::Neither,
            1 => Stay::Destination,
            2 => Stay::Source,
            _ => bail!("scroll stay must be ststNoStay, ststStayDest or ststStaySrc"),
        };
        let extent = match from {
            From::Left | From::Right => size.0,
            From::Top | From::Bottom => size.1,
        };
        ensure!(
            size.0 > 0 && size.1 > 0,
            "scroll dimensions must be positive"
        );
        Ok(Self { from, stay, extent })
    }

    pub(super) fn extent(&self) -> i32 {
        self.extent
    }

    pub(super) fn blend(&self, destination: &mut Image, source: &Image, phase: i32) {
        // Horizontal motion uses one band per row. Vertical motion uses one
        // band for the image, with a complete row as its translation unit.
        let (band_bytes, unit_bytes, edge) = match self.from {
            From::Left => (destination.width as usize * 4, 4, Edge::Low),
            From::Right => (destination.width as usize * 4, 4, Edge::High),
            From::Top => (
                destination.rgba.len(),
                destination.width as usize * 4,
                Edge::Low,
            ),
            From::Bottom => (
                destination.rgba.len(),
                destination.width as usize * 4,
                Edge::High,
            ),
        };
        let shift = phase as usize * unit_bytes;
        for (destination, source) in destination
            .rgba
            .chunks_exact_mut(band_bytes)
            .zip(source.rgba.chunks_exact(band_bytes))
        {
            let remaining = band_bytes - shift;
            match edge {
                Edge::Low => {
                    if !matches!(self.stay, Stay::Destination) {
                        destination.copy_within(..remaining, shift);
                    }
                    let incoming = if matches!(self.stay, Stay::Source) {
                        &source[..shift]
                    } else {
                        &source[remaining..]
                    };
                    destination[..shift].copy_from_slice(incoming);
                }
                Edge::High => {
                    if !matches!(self.stay, Stay::Destination) {
                        destination.copy_within(shift.., 0);
                    }
                    let incoming = if matches!(self.stay, Stay::Source) {
                        &source[remaining..]
                    } else {
                        &source[..shift]
                    };
                    destination[remaining..].copy_from_slice(incoming);
                }
            }
        }
    }
}
