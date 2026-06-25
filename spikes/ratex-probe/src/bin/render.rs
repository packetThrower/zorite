//! RaTeX *raster* render probe — proves the turnkey path for gpui: RaTeX
//! rasterizes a formula to a PNG/Pixmap (pure-Rust, via `ratex-render` →
//! tiny-skia), which a gpui host hands to `RenderImage` exactly like zorite's
//! Mermaid/PDF pages. Writes a PNG per formula to /tmp/ratex-out/.

use ratex_layout::{layout, to_display_list, LayoutOptions};
use ratex_parser::parse;
use ratex_render::{render_to_png, RenderOptions};

fn main() {
    let out = "/tmp/ratex-out";
    std::fs::create_dir_all(out).expect("mkdir");

    let samples = [
        ("integral", r"\int_0^1 4x \, dx"),
        ("quadratic", r"x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}"),
        ("basel", r"\sum_{n=1}^{\infty} \frac{1}{n^2} = \frac{\pi^2}{6}"),
        ("euler", r"e^{i\pi} + 1 = 0"),
        (
            "maxwell",
            r"\nabla \times \vec{B} = \mu_0 \vec{J} + \mu_0 \varepsilon_0 \frac{\partial \vec{E}}{\partial t}",
        ),
    ];

    // 44px em at 2x device-pixel-ratio → crisp HiDPI output, white background.
    let opts = RenderOptions {
        font_size: 44.0,
        device_pixel_ratio: 2.0,
        ..Default::default()
    };

    for (name, latex) in samples {
        match render_one(latex, &opts) {
            Ok(png) => {
                let path = format!("{out}/{name}.png");
                std::fs::write(&path, &png).expect("write png");
                println!("✓ {name:<10} {path}  ({} bytes)", png.len());
            }
            Err(e) => println!("✗ {name:<10} {e}"),
        }
    }
}

fn render_one(latex: &str, opts: &RenderOptions) -> Result<Vec<u8>, String> {
    let nodes = parse(latex).map_err(|e| format!("parse: {e:?}"))?;
    let lbox = layout(&nodes, &LayoutOptions::default());
    let dl = to_display_list(&lbox);
    render_to_png(&dl, opts)
}
