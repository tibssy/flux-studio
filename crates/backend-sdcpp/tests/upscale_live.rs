//! Live upscaler checks. They need `sd-server` and downloaded weights:
//!
//! ```text
//! FLUX_SD_SERVER=vendor/stable-diffusion.cpp/build/bin/sd-server \
//! FLUX_MODELS_DIR=/tmp/upscale-models \
//! FLUX_UPSCALE_INPUT=/tmp/lr.png \
//! cargo test -p flux-backend-sdcpp --test upscale_live -- --ignored
//! ```

use std::path::PathBuf;

use flux_backend_sdcpp::{SdServerConfig, SdUpscaleServer};
use flux_core::upscaler::{DEFAULT_TILE_SIZE, UpscaleRequest};

#[test]
#[ignore = "requires sd-server and downloaded upscaler weights"]
fn upscales_at_4x_and_2x() {
    let cli = PathBuf::from(std::env::var("FLUX_SD_SERVER").expect("FLUX_SD_SERVER"));
    let models_dir = PathBuf::from(std::env::var("FLUX_MODELS_DIR").expect("FLUX_MODELS_DIR"));
    let input = PathBuf::from(std::env::var("FLUX_UPSCALE_INPUT").expect("FLUX_UPSCALE_INPUT"));
    let output = std::env::temp_dir().join("flux-live-upscale-4x.png");
    let output_2x = std::env::temp_dir().join("flux-live-upscale-2x.png");

    let server = SdUpscaleServer::new(SdServerConfig {
        cli,
        models_dir,
        backend: None,
    });

    let four = server
        .upscale(&UpscaleRequest {
            input: input.clone(),
            output: output.clone(),
            upscaler_id: "mosr-4x".into(),
            factor: 4,
            tile_size: DEFAULT_TILE_SIZE,
        })
        .expect("4x upscale");
    let (input_width, input_height) = image::image_dimensions(&input).unwrap();
    assert_eq!(four.width, input_width * 4);
    assert_eq!(four.height, input_height * 4);
    assert!(output.exists());

    let two = server
        .upscale(&UpscaleRequest {
            input,
            output: output_2x.clone(),
            upscaler_id: "mosr-4x".into(),
            factor: 2,
            tile_size: DEFAULT_TILE_SIZE,
        })
        .expect("2x upscale");
    assert_eq!(two.width, input_width * 2);
    assert_eq!(two.height, input_height * 2);
    assert!(output_2x.exists());

    let _ = std::fs::remove_file(output);
    let _ = std::fs::remove_file(output_2x);
}
