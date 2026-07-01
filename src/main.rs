// ═══════════════════════════════════════════════════════════════════════════════
// IMPORTS
// ═══════════════════════════════════════════════════════════════════════════════

use bytes;
use clap::Parser;
use gpui::*;
use gpui_component::{
    Root, Theme, ThemeMode, h_flex,
    text::{TextView, TextViewState},
};
use gpui_component_assets::Assets;
use reqwest::header::USER_AGENT;
use scraper::{Html, Selector};
use url::Url;

#[derive(Parser, Debug)]
struct Cli {
    #[clap(default_value = "./README.md")]
    source: String,
    // Process GitHub trending pages instead of viewing a markdown file
    #[clap(long, default_value_t = false)]
    github_trending: bool,
}

// ═══════════════════════════════════════════════════════════════════════════════
// GPUI APPLICATION
// ═══════════════════════════════════════════════════════════════════════════════

actions!(app, [Quit]);

fn quit(_: &Quit, cx: &mut gpui::App) {
    cx.quit();
}

use futures::{AsyncReadExt, future::join_all};
use gpui::http_client::{AsyncBody, HttpClient};
use reqwest_client::ReqwestClient;
use std::sync::{Arc, Mutex};

fn main() {
    let cli = Cli::parse();

    gpui_platform::application()
        .with_assets(Assets)
        .with_http_client(Arc::new(ReqwestClient::new()))
        .run(move |cx| {
            if cli.github_trending {
                trendy(cx);
                return;
            }
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

// ═══════════════════════════════════════════════════════════════════════════════
// GITHUB TRENDING - SCRAPE GITHUB TRENDING REPOS
// ═══════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
struct TrendyRepoData {
    owner: String,
    repo_name: String,
    default_branch: String,
    readme_filenames: Vec<String>,
}

async fn trendy_scrape_repo_page(
    client: Arc<dyn HttpClient>,
    owner_repo: String,
    data_pool: Arc<Mutex<Vec<TrendyRepoData>>>,
) {
    let repo_url = format!("https://github.com/{}", owner_repo);
    let result = client.get(&repo_url, AsyncBody::empty(), true).await;

    let response = match result {
        Ok(res) if res.status().is_success() => res,
        _ => return,
    };

    let mut body_stream = response.into_body();
    let mut html_string = String::new();
    if body_stream.read_to_string(&mut html_string).await.is_err() {
        return;
    }

    let repo_doc = Html::parse_document(&html_string);

    let branch_sel = Selector::parse("#ref-picker-repos-header-ref-selector > span > span.prc-Button-Label-FWkx3 > div > div.ref-selector-button-text-container.RefSelectorAnchoredOverlay-module__RefSelectorBtnTextContainer__Di3rk > span").unwrap();
    let owner_sel = Selector::parse("body > div.logged-in.env-production.page-responsive > div.position-relative.header-wrapper.js-header-wrapper > react-partial:nth-child(9) > div > header > div.prc-Stack-Stack-UQ9k6 > div.styles-module__center__R3QRv.styles-module__withLocalNavigation__rjTJ_.prc-Stack-Stack-UQ9k6 > nav > ol > li:nth-child(1) > a > span").unwrap();
    let name_sel = Selector::parse("body > div.logged-in.env-production.page-responsive > div.position-relative.header-wrapper.js-header-wrapper > react-partial:nth-child(9) > div > header > div.prc-Stack-Stack-UQ9k6 > div.styles-module__center__R3QRv.styles-module__withLocalNavigation__rjTJ_.prc-Stack-Stack-UQ9k6 > nav > ol > li:nth-child(2) > a").unwrap();

    // Use your working file list row CSS selector target
    let file_selector = Selector::parse("a.Link--primary").unwrap();

    let extracted_owner = repo_doc
        .select(&owner_sel)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_else(|| owner_repo.split('/').next().unwrap_or("").to_string());

    let extracted_name = repo_doc
        .select(&name_sel)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_else(|| owner_repo.split('/').nth(1).unwrap_or("").to_string());

    let extracted_branch = repo_doc
        .select(&branch_sel)
        .next()
        .map(|el| el.text().collect::<Vec<_>>().join("").trim().to_string())
        .unwrap_or_else(|| "main".to_string());

    // 🏎️ YOUR PROTOTYPE LOGIC: Pulling down ALL files into a clean vector array
    let mut collected_readmes: Vec<String> = repo_doc
        .select(&file_selector)
        .filter_map(|element| {
            if !element
                .value()
                .attr("href")
                .unwrap_or("")
                .contains("/blob/")
            {
                return None;
            }

            let txt1 = element.text().next()?;

            txt1.as_bytes()
                .windows(6)
                .any(|window| {
                    window
                        .iter()
                        .map(|b| b.to_ascii_lowercase())
                        .eq("readme".bytes())
                })
                .then_some(txt1.trim().to_string())
        })
        .collect();

    // 💡 FIX THE DUPES: Sort and filter out contiguous twin elements!
    collected_readmes.sort();
    collected_readmes.dedup(); // Drops twins cleanly with zero extra dependencies

    let entry = TrendyRepoData {
        owner: extracted_owner,
        repo_name: extracted_name,
        default_branch: extracted_branch,
        readme_filenames: collected_readmes,
    };
    data_pool.lock().unwrap().push(entry);
}

async fn trendy_fetch_trending_timelines(
    client: Arc<dyn HttpClient>,
    executor: gpui::BackgroundExecutor,
    endpoints: Vec<(&'static str, &'static str)>,
    dedup_set: Arc<Mutex<std::collections::HashSet<String>>>,
    sub_tasks: Arc<Mutex<Vec<gpui::Task<()>>>>,
    data_pool: Arc<Mutex<Vec<TrendyRepoData>>>,
) {
    let mut tasks = Vec::new();

    for (timeline, _) in endpoints {
        let http_client = client.clone();
        let shared_set = dedup_set.clone();
        let exec = executor.clone();
        let task_tracker = sub_tasks.clone();
        let final_pool = data_pool.clone();

        // Spawn master trackers concurrently
        let task = executor.spawn(async move {
            let url = format!("https://github.com/trending?since={}", timeline);
            let result = http_client.get(&url, AsyncBody::empty(), true).await;

            let response = match result {
                Ok(res) if res.status().is_success() => res,
                _ => return,
            };

            let mut body_stream = response.into_body();
            let mut html_string = String::new();
            if body_stream.read_to_string(&mut html_string).await.is_err() {
                return;
            }

            {
                let document = Html::parse_document(&html_string);
                let row_selector = Selector::parse("article.Box-row h2 a").unwrap();

                for element in document.select(&row_selector) {
                    // let text = element.text().collect::<Vec<_>>().join("");
                    let owner_repo = element.text().collect::<Vec<_>>().join("").replace("\n", "").replace(" ", "");

                    // Real-time stream pipelining deduplication
                    let is_new = shared_set.lock().unwrap().insert(owner_repo.clone());

                    if is_new {
                        let sub_client = http_client.clone();
                        let target_repo = owner_repo.clone();
                        let pool_pointer = final_pool.clone();

                        // Launch sub-scraper onto pool and anchor it immediately
                        let sub_task =
                            exec.spawn(trendy_scrape_repo_page(sub_client, target_repo, pool_pointer));
                        task_tracker.lock().unwrap().push(sub_task);
                    }
                }
            }
        });
        tasks.push(task);
    }

    // Block until timelines finish row parsing
    join_all(tasks).await;
}

fn trendy(cx: &mut gpui::App) {
    let http_client = cx.http_client();
    let endpoints = vec![
        ("daily", "https://github.com"),
        ("weekly", "https://github.com"),
        ("monthly", "https://github.com"),
    ];

    cx.spawn(move |cx: &mut gpui::AsyncApp| {
        let cx_owned = cx.clone();

        async move {
            let executor = cx_owned.background_executor();

            // Central thread-safe aggregators
            let dedup_set = Arc::new(Mutex::new(std::collections::HashSet::new()));
            let deep_scrape_tasks = Arc::new(Mutex::new(Vec::new()));
            let accumulated_repos = Arc::new(Mutex::new(Vec::<TrendyRepoData>::new()));

            // Step 1: Run timeline trackers to completion
            trendy_fetch_trending_timelines(
                http_client,
                executor.clone(),
                endpoints,
                dedup_set,
                deep_scrape_tasks.clone(),
                accumulated_repos.clone(),
            )
            .await;

            // Step 2: Extract and await all real-time pipelined sub-scrapers
            let final_scrapers = std::mem::take(&mut *deep_scrape_tasks.lock().unwrap());
            join_all(final_scrapers).await;

            println!("\n==================================================");
            println!("   🎉 FINAL SORTED TRENDING DATA (WITH ALL READMES)");
            println!("==================================================");

            {
                let mut dataset = accumulated_repos.lock().unwrap();
                dataset.sort_by(|a, b| {
                    a.owner
                        .to_lowercase()
                        .cmp(&b.owner.to_lowercase())
                        .then_with(|| a.repo_name.to_lowercase().cmp(&b.repo_name.to_lowercase()))
                });

                println!("Gathered a total of {} unique items:\n", dataset.len());
                for (idx, repo) in dataset.iter().enumerate() {
                    // Turn our array slice back into a comma-separated display block string
                    let readmes_list = if repo.readme_filenames.is_empty() {
                        "None found".to_string()
                    } else {
                        repo.readme_filenames.join(", ")
                    };

                    println!(
                        "[{:02}] Owner: {:<18} | Repo: {:<22} | Branch: {:<10} | Readmes: [{}]",
                        idx + 1,
                        repo.owner,
                        repo.repo_name,
                        repo.default_branch,
                        readmes_list
                    );
                }
            }
            println!("==================================================\n");

            // Safe shutdown
            cx_owned.update(|cx| {
                cx.quit();
            });
        }
    })
    .detach();
}

// ═══════════════════════════════════════════════════════════════════════════════
// GITHUB UTILITIES - REPO DISCOVERY, BRANCH DETECTION, README SCRAPING
// ═══════════════════════════════════════════════════════════════════════════════

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

// Git protocol packet line iterator for parsing git-upload-pack responses
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

// Extract default branch from git-upload-pack protocol response
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

// Load content from file or URL (blocking), handling GitHub repo URLs specially
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

            scrape_best_readme_url(url.clone(), branch_from_git_raw(&response.bytes().ok()?)?)
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

// Find the best README URL for a GitHub repo at a specific branch
fn scrape_best_readme_url(url: Url, branch: String) -> Option<String> {
    scrape_dir_for_readmes(url.clone())
        .ok()
        .map(|readmes| {
            eprintln!("{:?}", readmes);
            readmes
        })
        .and_then(|readmes| choose_best_readme(&readmes).map(|s| s.to_string()))
        .map(|chosen_readme| {
            format!(
                "https://raw.githubusercontent.com{}/{}/{}",
                url.path(),
                branch,
                chosen_readme
            )
        })
}

// Choose the best README from a list of filenames using heuristics
fn choose_best_readme(filenames: &Vec<String>) -> Option<&String> {
    // If we got 0 or 1, choose the (optional) first one
    if filenames.len() <= 1 {
        return filenames.first();
    }

    const STANDARD: &str = "README.md";

    // If the most-standard "README.md" is one of them, choose that.
    // TODO: It could be that README.md is not English but the repo's native language
    // TODO: and one of the other readmes is English - check for country/language codes in filenames
    if let Some(readme) = filenames.iter().find(|&f| f == STANDARD) {
        return Some(readme);
    }

    // If we got multiple, filter out any case variants of readme.md
    let (readme_md_any_case, other_readmes): (Vec<&String>, Vec<&String>) = filenames
        .iter()
        .partition(|f| f.eq_ignore_ascii_case(STANDARD));

    // If we have exactly one case variant of readme.md, choose it
    // TODO: It could be that the readme.md variant is not English but
    // TODO: the repo's native language and one of the other readmes is English
    // TODO:  - check for country/language codes in filenames
    if readme_md_any_case.len() == 1 {
        return Some(readme_md_any_case[0]);
    }

    // If we have no case variants of readme.md and we only have one other readme, choose it
    if readme_md_any_case.len() == 0 && other_readmes.len() == 1 {
        return Some(other_readmes[0]);
    }

    // We've got multiple variants of readme.md AND/OR multiple other readmes
    println!("    'readme.md' variants: {:?}", readme_md_any_case);
    println!("    Other readmes: {:?}", other_readmes);

    // Try partitioning by those with a .md file extension in any case vs others
    let (md_files, other_files): (Vec<&String>, Vec<&String>) = filenames
        .iter()
        .partition(|f| f.ends_with(".md") || f.ends_with(".MD"));

    // If we have exactly one .md file, choose it
    if md_files.len() == 1 {
        return Some(md_files[0]);
    }

    println!("    .md files: {:?}", md_files);
    println!("    Other files: {:?}", other_files);

    filenames.first()
}

// Scrape a GitHub directory and return a list of README filenames
fn scrape_dir_for_readmes<U: reqwest::IntoUrl>(
    url: U,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
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

    let file_selector = Selector::parse(
        ".react-directory-row-name-cell-large-screen .react-directory-truncate a.Link--primary",
    )
    .unwrap();

    let files: Vec<_> = document
        .select(&file_selector)
        .filter_map(|element| {
            // 1. Filter out directories by checking the link path structure
            if !element
                .value()
                .attr("href")
                .unwrap_or("")
                .contains("/blob/")
            {
                return None;
            }

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

#[cfg(test)]
mod tests {
    use url::Url;

    // ═══════════════════════════════════════════════════════════════════════════════
    // TESTS: GitHub URL Transformation
    // ═══════════════════════════════════════════════════════════════════════════════

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

    // ═══════════════════════════════════════════════════════════════════════════════
    // TESTS: Default Branch Detection
    // ═══════════════════════════════════════════════════════════════════════════════

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

    // ═══════════════════════════════════════════════════════════════════════════════
    // TESTS: README Scraping
    // ═══════════════════════════════════════════════════════════════════════════════

    use super::scrape_dir_for_readmes;

    #[test]
    fn scrape_single_dir() {
        let url = "https://github.com/hippietrail/orly";
        let files = scrape_dir_for_readmes(Url::parse(url).unwrap()).unwrap();
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
                let files = scrape_dir_for_readmes(url).unwrap();
                println!("{:?}", files);
            }
        }
        println!("========================================================\n");
    }

    // ═══════════════════════════════════════════════════════════════════════════════
    // TESTS: GitHub Trending Integration
    // ═══════════════════════════════════════════════════════════════════════════════

    use reqwest::blocking::Client;
    use scraper;

    #[test]
    fn scrape_trending_repos_from_github() {
        let mut pairs = Vec::new();
        let mut repos_total = 0;
        for since in ["daily", "weekly", "monthly"].iter() {
            let client = Client::new();
            let response = client
                .get(format!("https://github.com/trending?since={}", since))
                .send()
                .unwrap();
            let doc_str = response.text().unwrap();
            let html = scraper::Html::parse_document(&doc_str);
            let selector = scraper::Selector::parse("article.Box-row h2.h3 a").unwrap();
            let user_repo_pairs = html.select(&selector);

            for user_repo_pair in user_repo_pairs {
                repos_total += 1;
                let href = user_repo_pair.value().attr("href").unwrap().to_string();
                if !pairs.contains(&href) {
                    pairs.push(href);
                }
            }
        }

        println!(
            "Total pairs: {}, unique pairs: {}",
            repos_total,
            pairs.len()
        );
        println!("{:?}", pairs);

        let mut branch_tally = std::collections::HashMap::new();
        let mut readme_tally = std::collections::HashMap::new();
        for pair in pairs {
            println!("==== {} ====", pair);
            let url = Url::parse(&format!("https://github.com{}", pair)).unwrap();
            let discovery_url = into_discovery_url(&url).unwrap();
            let response = reqwest::blocking::get(discovery_url).unwrap();
            let branch = branch_from_git_raw(&response.bytes().unwrap()).unwrap();
            println!("  Branch: {}", branch);
            *branch_tally.entry(branch).or_insert(0) += 1;

            let readmes = scrape_dir_for_readmes(url).unwrap();
            println!("  Readmes: {:?}", readmes);
            for readme in readmes {
                *readme_tally.entry(readme).or_insert(0) += 1;
            }
        }
        // sort them both descending
        let mut branch_tally_vec: Vec<(String, i32)> = branch_tally.into_iter().collect();
        branch_tally_vec.sort_by(|a, b| b.1.cmp(&a.1));
        let mut readme_tally_vec: Vec<(String, i32)> = readme_tally.into_iter().collect();
        readme_tally_vec.sort_by(|a, b| b.1.cmp(&a.1));
        println!("\nBranch tally: {:?}", branch_tally_vec);
        println!("Readme tally: {:?}", readme_tally_vec);
    }
}
