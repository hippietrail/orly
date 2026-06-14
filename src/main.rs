use clap::Parser;
use gpui::*;
use gpui_component::{
    Root, Theme, ThemeMode, h_flex,
    text::{TextView, TextViewState},
};
use gpui_component_assets::Assets;

#[derive(Parser, Debug)]
struct Cli {
    #[clap(default_value = "./README.md")]
    source: String,
}

actions!(app, [Quit]);

fn quit(_: &Quit, cx: &mut gpui::App) {
    cx.quit();
}

fn main() {
    let cli = Cli::parse();
    
    gpui_platform::application().with_assets(Assets).run(move |cx| {
        gpui_component::init(cx);
        cx.on_action(quit);
        cx.bind_keys(vec![KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![Menu::new("File").items(vec![MenuItem::action("Quit", Quit)])]);

        cx.on_window_closed(|cx, _w| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.0), px(720.0)), cx)),
            show: true,
            ..Default::default()
        };

        cx.open_window(window_options, |window, cx| {
            Theme::change(ThemeMode::Dark, Some(window), cx);
            
            let source_str = cli.source.clone();
            let view = cx.new(|cx| {
                let markdown_state = cx.new(|cx| TextViewState::markdown("# Loading...", cx));
                
                cx.spawn(async move |view_handle: WeakEntity<App>, cx: &mut AsyncApp| {
                    // Use std::thread::spawn to bypass Tokio executor constraints completely
                    let content = cx.background_executor()
                        .spawn(async move {
                            std::thread::spawn(move || load_content_blocking(&source_str))
                                .join()
                                .unwrap_or_else(|_| "# Thread Panic\n\nFailed to join network fetch thread.".into())
                        })
                        .await;
                        
                    let _ = view_handle.update(cx, |this: &mut App, cx| {
                        this.markdown.update(cx, |tvs, cx| {
                            tvs.set_text(content.as_str(), cx);
                        });
                    });
                }).detach();
                
                App { markdown: markdown_state }
            });

            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}

// Uses reqwest's blocking client interface 
fn load_content_blocking(source: &str) -> SharedString {
    if source.starts_with("http://") || source.starts_with("https://") {
        match reqwest::blocking::get(source) {
            Ok(response) => match response.text() {
                Ok(text) => text.into(),
                Err(err) => format!("# Error Reading Response\n\nFailed to parse URL content: **{}**", err).into(),
            },
            Err(err) => format!("# Network Error\n\nCould not fetch `{}`: **{}**", source, err).into(),
        }
    } else {
        std::fs::read_to_string(source).unwrap_or_else(|err| {
            format!("# Error Loading File\n\nCould not read `{}`: **{}**", source, err)
        }).into()
    }
}

struct App {
    markdown: Entity<TextViewState>,
}

impl Render for App {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(TextView::new(&self.markdown).scrollable(true))
    }
}
