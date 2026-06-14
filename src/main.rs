use bytes;
use clap::Parser;
use gpui::*;
use gpui_component::{
    Root, Theme, ThemeMode, h_flex,
    text::{TextView, TextViewState},
};
use gpui_component_assets::Assets;
use url::Url;

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

    gpui_platform::application()
        .with_assets(Assets)
        .run(move |cx| {
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

                let source_str = cli.source.clone();
                let view = cx.new(|cx| {
                    let markdown_state = cx.new(|cx| TextViewState::markdown("# Loading...", cx));

                    cx.spawn(
                        async move |view_handle: WeakEntity<App>, cx: &mut AsyncApp| {
                            // Use std::thread::spawn to bypass Tokio executor constraints completely
                            let content = cx
                                .background_executor()
                                .spawn(async move {
                                    std::thread::spawn(move || load_content_blocking(&source_str))
                                        .join()
                                        .unwrap_or_else(|_| {
                                            "# Thread Panic\n\nFailed to join network fetch thread."
                                                .into()
                                        })
                                })
                                .await;

                            let _ = view_handle.update(cx, |this: &mut App, cx| {
                                this.markdown.update(cx, |tvs, cx| {
                                    tvs.set_text(content.as_str(), cx);
                                });
                            });
                        },
                    )
                    .detach();

                    App {
                        markdown: markdown_state,
                    }
                });

                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");

            cx.activate(true);
        });
}

fn to_github_repo_url(url: &Url) -> Option<String> {
    url.path_segments()
        .and_then(|mut s| match (s.next(), s.next(), s.next(), s.next()) {
            (Some(user), Some(repo), None, _) | (Some(user), Some(repo), Some(""), None)
                if !user.is_empty() && !repo.is_empty() =>
            {
                Some(format!(
                    "https://github.com/{user}/{repo}.git/info/refs?service=git-upload-pack"
                ))
            }
            _ => None,
        })
}

struct PktLineIterator<'a> {
    data: &'a [u8],
}

impl<'a> Iterator for PktLineIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.len() < 4 {
            return None;
        }

        let len =
            usize::from_str_radix(String::from_utf8_lossy(&self.data[..4]).trim(), 16).unwrap_or(0);

        match len {
            0..=3 => {
                self.data = &self.data[4..]; // Skip over control/flush packets
                self.next() // Recurse to find the next valid data packet
            }
            _ if len <= self.data.len() => {
                let payload = &self.data[4..len];
                self.data = &self.data[len..]; // Advance the iterator past this packet
                Some(payload)
            }
            _ => None, // Malformed packet length exceeds remaining data
        }
    }
}

fn branch_from_git_raw(raw: &bytes::Bytes) -> Option<String> {
    PktLineIterator { data: raw }
        .filter(|payload| payload.iter().filter(|&&b| b == 0).count() == 1)
        .find_map(|payload| {
            let nul_pos = payload.iter().position(|&b| b == 0)?;

            String::from_utf8_lossy(&payload[nul_pos + 1..])
                .split_whitespace()
                .find_map(|cap| {
                    cap.strip_prefix("symref=HEAD:")?
                        .strip_prefix("refs/heads/")
                        .map(String::from)
                })
        })
}

fn load_content_blocking(source: &str) -> SharedString {
    if let Ok(url) = Url::parse(source)
        && ["https", "http"].contains(&url.scheme())
    {
        let source = (|| {
            let domain = url.domain()?;
            if !["github.com", "www.github.com"].contains(&domain) {
                return None;
            }

            let github_repo_url = to_github_repo_url(&url)?;
            eprintln!("GitHub repo URL: {}", github_repo_url);

            let response = reqwest::blocking::get(&github_repo_url)
                .map_err(|err| println!("Error from `{}`: {}", github_repo_url, err))
                .ok()?;

            let branch = branch_from_git_raw(&response.bytes().ok()?)?;

            Some(format!(
                "https://raw.githubusercontent.com{}/{}/README.md",
                url.path(),
                branch
            ))
        })()
        .unwrap_or_else(|| source.to_owned());

        match reqwest::blocking::get(&source) {
            Ok(response) => match response.text() {
                Ok(text) => text.into(),
                Err(err) => format!(
                    "# Error Reading Response\n\nFailed to parse URL content: **{}**",
                    err
                )
                .into(),
            },
            Err(err) => format!(
                "# Network Error\n\nCould not fetch `{}`: **{}**",
                source, err
            )
            .into(),
        }
    } else {
        std::fs::read_to_string(source)
            .unwrap_or_else(|err| {
                format!(
                    "# Error Loading File\n\nCould not read `{}`: **{}**",
                    source, err
                )
            })
            .into()
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
