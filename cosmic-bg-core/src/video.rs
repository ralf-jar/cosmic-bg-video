use gstreamer::prelude::*;
use gstreamer_app::AppSink;

pub fn init_pipeline(uri: &str) -> (gstreamer::Pipeline, AppSink) {
    gstreamer::init().expect("GStreamer init failed");

    let pipeline = gstreamer::ElementFactory::make("playbin").build().unwrap();
    pipeline.set_property("uri", uri);

    let sink_bin = gstreamer::parse::bin_from_description(
        "videoconvert ! videoscale ! videoconvert ! video/x-raw,format=BGRA ! appsink name=sink emit-signals=false sync=true max-buffers=1 drop=true",
        true,
    ).expect("Error creando bin de video");

    pipeline.set_property("video-sink", &sink_bin);
    pipeline.set_property("audio-sink", &gstreamer::ElementFactory::make("fakesink").build().unwrap());

    let appsink = sink_bin.by_name("sink").unwrap().downcast::<AppSink>().unwrap();
    (pipeline.downcast::<gstreamer::Pipeline>().unwrap(), appsink)
}