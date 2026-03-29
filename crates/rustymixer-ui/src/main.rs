use dioxus::prelude::*;

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("Starting RustyMixer");
    dioxus::launch(app);
}

fn app() -> Element {
    rsx! {
        div {
            style: "font-family: sans-serif; padding: 20px; text-align: center;",
            h1 { "RustyMixer" }
            p { "Professional DJ application — coming soon." }
        }
    }
}
