use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use flate2::write::GzEncoder;
use flate2::Compression;

fn main() {
    println!("cargo:rerun-if-changed=static/script.js");
    println!("cargo:rerun-if-changed=static/style.css");
    println!("cargo:rerun-if-changed=static/index.html");

    let out_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let static_dir = Path::new(&out_dir).join("static");

    let js_path = static_dir.join("script.js");
    let gz_js_path = static_dir.join("script.js.gz");
    if let Ok(js_code) = fs::read_to_string(&js_path) {
        let minified_js = minifier::js::minify(&js_code);
        compress_with_gzip(&minified_js.to_string().as_bytes(), &gz_js_path).expect("Failed to gzip js");
    }

    let css_path = static_dir.join("style.css");
    let gz_css_path = static_dir.join("style.css.gz");
    if let Ok(css_code) = fs::read_to_string(&css_path) {
        let minified_css = minifier::css::minify(&css_code).expect("CSS minify failed");
        compress_with_gzip(&minified_css.to_string().as_bytes(), &gz_css_path).expect("Failed to gzip css");
    }

    let html_path = static_dir.join("index.html");
    let gz_html_path = static_dir.join("index.html.gz");
    if let Ok(html_content) = fs::read(&html_path) {
        compress_with_gzip(&html_content, &gz_html_path).expect("Failed to gzip html");
    }
}

fn compress_with_gzip(data: &[u8], output_path: &Path) -> io::Result<()> {
    let output_file = fs::File::create(output_path)?;
    let mut encoder = GzEncoder::new(output_file, Compression::best());
    encoder.write_all(data)?;
    Ok(())
}
