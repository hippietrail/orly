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

// Converts a GitHub repository URL to its Git HTTP Smart Protocol discovery URL.
fn into_discovery_url(repo_url: &Url) -> Option<Url> {
    repo_url
        .path_segments()
        .and_then(|mut s| match (s.next(), s.next(), s.next(), s.next()) {
            (Some(user), Some(repo), None, _) | (Some(user), Some(repo), Some(""), None)
                if !user.is_empty() && !repo.is_empty() =>
            {
                // Git HTTP Smart Protocol URL
                let mut endpoint_url = repo_url.clone();
                endpoint_url.set_path(&format!("/{user}/{repo}.git/info/refs"));
                endpoint_url.set_query(Some("service=git-upload-pack"));
                Some(endpoint_url)
            }
            _ => None,
        })
}

pub struct PktLineIterator<'a> {
    pub data: &'a [u8],
}

impl<'a> Iterator for PktLineIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.data.is_empty() {
            return None;
        }

        // 1. Every packet must have at least a 4-byte hex length prefix
        if self.data.len() < 4 {
            self.data = &[]; // Consume remaining malformed bytes
            return None;
        }

        // 2. Read the 4-byte hex string
        let total_len = usize::from_str_radix(str::from_utf8(&self.data[..4]).ok()?, 16).ok()?;

        // 3. Handle special Git control packets
        // Flush packet (0000) - skip it and continue
        if total_len == 0 {
            self.data = &self.data[4..];
            return self.next(); // Recursively call to get next packet
        }

        // Delimiter packet (0001) - skip it and continue
        if total_len == 1 {
            self.data = &self.data[4..];
            return self.next(); // Recursively call to get next packet
        }

        // Safety check for malformed server packets
        if total_len < 4 || total_len > self.data.len() {
            self.data = &[];
            return None;
        }

        // 4. Extract the payload data (Strips the 4-byte length prefix!)
        let payload = &self.data[4..total_len];

        // 5. Advance the iterator window past this packet
        self.data = &self.data[total_len..];

        Some(payload)
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
        // Source is a URL, check if it's a GitHub URL
        let source = (|| {
            let domain = url.domain()?;
            if !["github.com", "www.github.com"].contains(&domain) {
                return None;
            }

            let discovery_url = into_discovery_url(&url)?.to_string();
            eprintln!("GitHub repo URL: {}", discovery_url);

            let response = reqwest::blocking::get(&discovery_url)
                .map_err(|err| println!("Error from `{}`: {}", discovery_url, err))
                .ok()?;

            let branch = branch_from_git_raw(&response.bytes().ok()?)?;

            // TODO: we know the branch but we do NOT know the readme file name
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

#[cfg(test)]
mod tests {
    use url::Url;

    // Transforming GitHub repo URLs

    use super::into_discovery_url;

    #[test]
    fn retroghidra_no_trailing_slash() {
        let url = Url::parse("https://github.com/hippietrail/retroghidra").unwrap();
        let discovery_url = into_discovery_url(&url).unwrap();
        assert_eq!(
            discovery_url.to_string(),
            "https://github.com/hippietrail/retroghidra.git/info/refs?service=git-upload-pack"
        );
    }

    #[test]
    fn retroghidra_with_trailing_slash() {
        let url = Url::parse("https://github.com/hippietrail/retroghidra/").unwrap();
        let discovery_url = into_discovery_url(&url).unwrap();
        assert_eq!(
            discovery_url.to_string(),
            "https://github.com/hippietrail/retroghidra.git/info/refs?service=git-upload-pack"
        );
    }

    #[test]
    fn fail_username_without_repo_no_trailing_slash() {
        let url = Url::parse("https://github.com/hippietrail").unwrap();
        let discovery_url = into_discovery_url(&url);
        assert!(discovery_url.is_none());
    }

    #[test]
    fn fail_username_without_repo_with_trailing_slash() {
        let url = Url::parse("https://github.com/hippietrail/").unwrap();
        let discovery_url = into_discovery_url(&url);
        assert!(discovery_url.is_none());
    }

    #[test]
    fn fail_too_many_path_segments() {
        let url = Url::parse("https://github.com/foo/bar/baz").unwrap();
        let discovery_url = into_discovery_url(&url);
        assert!(discovery_url.is_none());
    }

    // Finding the default branch

    use crate::branch_from_git_raw;

    #[test]
    fn retroghidra_uses_main() {
        let url = Url::parse("https://github.com/hippietrail/retroghidra").unwrap();
        let discovery_url = into_discovery_url(&url).unwrap();
        let response = reqwest::blocking::get(discovery_url).unwrap();
        let branch = branch_from_git_raw(&response.bytes().unwrap()).unwrap();
        assert_eq!(branch, "main");
    }

    #[test]
    fn harper_uses_master() {
        let url = Url::parse("https://github.com/automattic/harper").unwrap();
        let discovery_url = into_discovery_url(&url).unwrap();
        let response = reqwest::blocking::get(discovery_url).unwrap();
        let branch = branch_from_git_raw(&response.bytes().unwrap()).unwrap();
        assert_eq!(branch, "master");
    }

    #[test]
    fn fail_random_url() {
        let url = Url::parse("https://example.com").unwrap();
        let discovery_url = into_discovery_url(&url);
        assert!(discovery_url.is_none());
    }

    // Messing around

    use reqwest::header::USER_AGENT;
    use scraper::{Html, Selector};

    fn scrape_dir<U: reqwest::IntoUrl>(url: U) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        // 1. Create a client and fetch the HTML (Must include a User-Agent)
        let client = reqwest::blocking::Client::new();
        let response = client
            .get(url)
            .header(
                USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) RustScraper/1.0",
            )
            .send()?
            .text()?;

        // 2. Parse the HTML document
        let document = Html::parse_document(&response);

        // 2.5 get the default branch - `#ref-picker-repos-header-ref-selector > span > span.prc-Button-Label-FWkx3 > div > div.ref-selector-button-text-container.RefSelectorAnchoredOverlay-module__RefSelectorBtnTextContainer__Di3rk > span`
        let def_branch_selector = Selector::parse("#ref-picker-repos-header-ref-selector > span > span.prc-Button-Label-FWkx3 > div > div.ref-selector-button-text-container.RefSelectorAnchoredOverlay-module__RefSelectorBtnTextContainer__Di3rk > span").unwrap();
        if let Some(element) = document.select(&def_branch_selector).next() {
            let def_branch = element.text().collect::<Vec<_>>().join("");
            println!("\nDefault Branch: {}", def_branch.trim());
        }

        // 3. Extract the repository name using CSS Selectors
        let title_selector = Selector::parse("strong.mr-2 a").unwrap();
        if let Some(element) = document.select(&title_selector).next() {
            let repo_name = element.text().collect::<Vec<_>>().join("");
            println!("Repository Name: {}", repo_name.trim());
        }

        // 4. Extract the list of files/folders in the root directory
        // Explicitly target the desktop-only name cells to avoid mobile layout duplication
        let file_selector = Selector::parse(
            ".react-directory-row-name-cell-large-screen .react-directory-truncate a.Link--primary",
        )
        .unwrap();

        let files: Vec<_> = document
            .select(&file_selector)
            .filter_map(|element| {
                let txt1 = element.text().next()?;

                // Perform a zero-allocation, case-insensitive 'contains' check
                txt1.as_bytes()
                    .windows(6)
                    .any(|window| {
                        window
                            .iter()
                            .map(|b| b.to_ascii_lowercase())
                            .eq("readme".bytes())
                    })
                    .then_some(txt1)
            })
            .collect::<Vec<_>>();

        Ok(files.into_iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn scrape_single_dir() {
        let url = "https://github.com/hippietrail/orly";
        let files = scrape_dir(Url::parse(url).unwrap()).unwrap();
        println!("{:?}", files);
    }

    #[test]
    fn scrape_several_dirs() {
        let user_repos_to_test = &[
            ("hippietrail", "orly"),
            ("tinyhumansai", "openhuman"),
            ("Automattic", "harper"),
            ("FortAwesome", "Font-Awesome"),
            ("Kong", "insomnia"),
            ("Lightricks", "LTX-2"),
            ("zai-org", "GLM-5"),
            ("BuilderIO", "agent-native"),
            ("aishwaryanr", "awesome-generative-ai-guide"),
        ];

        println!("\n================ TESTING USER REPOS ================");
        for (user, repo) in user_repos_to_test {
            if let Ok(mut url) = Url::parse("https://github.com") {
                let path = &[user.to_owned(), repo.to_owned()].join("/");
                url.set_path(path);
                let files = scrape_dir(url).unwrap();
                println!("{:?}", files);
            }
        }
        println!("========================================================\n");
    }
}
