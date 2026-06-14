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
    filename: String,
}

actions!(app, [Quit]);

fn quit(_: &Quit, cx: &mut gpui::App) {
    cx.quit();
}

fn main() {
    let cli = Cli::parse();

    let file_string = std::fs::read_to_string(&cli.filename).unwrap_or_else(|err| {
        format!(
            "# Error Loading File\n\nCould not read `{}`: **{}**",
            cli.filename, err
        )
    });

    let content = SharedString::from(file_string);

    gpui_platform::application().with_assets(Assets).run(|cx| {
        gpui_component::init(cx);

        cx.on_action(quit);
        cx.bind_keys(vec![KeyBinding::new("cmd-q", Quit, None)]);
        cx.set_menus(vec![
            Menu::new("File").items(vec![MenuItem::action("Quit", Quit)]),
        ]);
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

            let view = cx.new(|cx| App::new(window, cx, content));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");

        cx.activate(true);
    });
}

struct App {
    markdown: Entity<TextViewState>,
}

impl App {
    pub fn new(_: &mut Window, cx: &mut Context<Self>, content: SharedString) -> Self {
        Self {
            markdown: cx.new(|cx| TextViewState::markdown(content.as_str(), cx)),
        }
    }
}

impl Render for App {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .child(TextView::new(&self.markdown).scrollable(true))
    }
}
