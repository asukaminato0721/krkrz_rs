//! Bounded affine sampling. Pixel blending uses the existing Layer operations.
use super::model::{Draw, Matrix};
use anyhow::{Result, ensure};
use krkrz_assets::media::Image;
use krkrz_tjs::unsupported;

pub(crate) struct Prepared<'a> {
    draw: &'a Draw,
    inverse: Matrix,
    bounds: [i32; 4],
}

pub(super) fn prepare<'a>(
    commands: &'a [Draw],
    clip: [i32; 4],
    budget: &mut u64,
) -> Result<Vec<Prepared<'a>>> {
    let mut output = Vec::new();
    let mut work = 0u64;
    for command in commands {
        let [a, b, c, d, x, y] = command.matrix.0;
        ensure!(
            command
                .matrix
                .0
                .iter()
                .all(|n| n.is_finite() && n.abs() <= 16384.0),
            "motion transform exceeds coordinate range"
        );
        let determinant = a * d - b * c;
        if determinant != 0.0 && command.opacity > 0.0 {
            ensure!(
                determinant.is_finite(),
                "invalid motion transform determinant"
            );
            let inverse = Matrix([
                d / determinant,
                -b / determinant,
                -c / determinant,
                a / determinant,
                (c * y - d * x) / determinant,
                (b * x - a * y) / determinant,
            ]);
            ensure!(
                inverse.0.iter().all(|n| n.is_finite()),
                "invalid inverse motion transform"
            );
            let image = &command.sprite.image;
            let corners = [
                [0.0, 0.0],
                [f64::from(image.width), 0.0],
                [0.0, f64::from(image.height)],
                [f64::from(image.width), f64::from(image.height)],
            ]
            .map(|[x, y]| command.matrix.point(x, y));
            ensure!(
                corners
                    .iter()
                    .flatten()
                    .all(|n| n.is_finite() && n.abs() <= 16384.0),
                "motion bounds exceed coordinate range"
            );
            let bounds = [
                (corners
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::INFINITY, f64::min)
                    .floor() as i32)
                    .max(clip[0]),
                (corners
                    .iter()
                    .map(|p| p[1])
                    .fold(f64::INFINITY, f64::min)
                    .floor() as i32)
                    .max(clip[1]),
                (corners
                    .iter()
                    .map(|p| p[0])
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil() as i32)
                    .min(clip[2]),
                (corners
                    .iter()
                    .map(|p| p[1])
                    .fold(f64::NEG_INFINITY, f64::max)
                    .ceil() as i32)
                    .min(clip[3]),
            ];
            if bounds[2] > bounds[0] && bounds[3] > bounds[1] {
                work = work
                    .checked_add((bounds[2] - bounds[0]) as u64 * (bounds[3] - bounds[1]) as u64)
                    .ok_or_else(|| unsupported("motion raster work overflow"))?;
                output.push(Prepared {
                    draw: command,
                    inverse,
                    bounds,
                });
            }
        }
    }
    *budget = budget
        .checked_sub(work.saturating_mul(4))
        .ok_or_else(|| unsupported("Motion.Player.draw execution budget exceeded"))?;
    Ok(output)
}

pub(super) fn render(image: &mut Image, commands: &[Prepared<'_>], face: i32, hold: bool) {
    for command in commands {
        let source = &command.draw.sprite.image;
        let opacity = command.draw.opacity.round().clamp(0.0, 255.0) as i32;
        let [left, top, right, bottom] = command.bounds;
        for y in top..bottom {
            for x in left..right {
                let [sx, sy] = command
                    .inverse
                    .point(f64::from(x) + 0.5, f64::from(y) + 0.5);
                if sx >= 0.0
                    && sy >= 0.0
                    && sx < f64::from(source.width)
                    && sy < f64::from(source.height)
                {
                    let offset = (sy as usize * source.width as usize + sx as usize) * 4;
                    let target = (y as usize * image.width as usize + x as usize) * 4;
                    let pixel = source.rgba[offset..offset + 4].try_into().unwrap();
                    crate::layer::blend_motion_pixel(
                        &mut image.rgba[target..target + 4],
                        pixel,
                        face,
                        opacity,
                        hold,
                    );
                }
            }
        }
    }
}
