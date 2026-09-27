use pretty_assertions::assert_eq;
use codel_tools::implementations::codel_build::image_gen::ImageGenConfig;
use codel_tools::implementations::codel_build::video_gen::VideoGenConfig;

use super::{MediaToolCredentials, image_gen_config, load_media_tool_config, video_gen_config};

fn config(toml: &str) -> super::Config {
    load_media_tool_config(&toml::from_str(toml).unwrap(), None).unwrap()
}




