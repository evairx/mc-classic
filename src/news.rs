//! The launcher's news pane, made from `assets/`: `news.html` and the files it links to, such as
//! the images in `assets/assets/`. The pane is a Swing `JEditorPane`, which renders simple HTML
//! and runs no JavaScript, so the infinite scroll lives in the agent (`NewsScroller.java`): the
//! first page holds the newest posts, and the agent appends `page/<n>` as the player nears the
//! bottom, until a page comes back empty.
//!
//! The posts are the `<div class="post ...">` blocks of `news.html`, newest first. Whatever lies
//! between them (the spacers) is dropped and put back between the posts each page holds.

use std::sync::OnceLock;

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

/// The launcher asks for `http://mcupdate.tumblr.com/`; its files live in `www/<DIR>`.
pub const DIR: &str = "mcupdate.tumblr.com";
/// Every file under `assets/`, as (path relative to it, contents). See build.rs.
pub const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/assets.rs"));

const PAGE: &str = include_str!("../assets/news.html");
/// Posts per page.
const PAGE_SIZE: usize = 5;
const SPACER: &str = "\n\n              <div class=\"spacer\"></div>\n\n              ";

struct News {
    /// `news.html` up to the first post.
    head: String,
    posts: Vec<String>,
    /// `news.html` after the last post, without its footer: the "Older" link went to Tumblr,
    /// and the agent loads older posts instead.
    tail: String,
}

fn news() -> &'static News {
    static NEWS: OnceLock<News> = OnceLock::new();
    NEWS.get_or_init(|| split(PAGE))
}

/// The first page: `news.html` with only the newest posts.
pub async fn first_page() -> Response {
    let news = news();
    let shown = &news.posts[..PAGE_SIZE.min(news.posts.len())];
    log!("GET noticias: {} de {} posts", shown.len(), news.posts.len());
    html(format!("{}{}{}", news.head, shown.join(SPACER), news.tail))
}

/// Posts of page `page` (2 onwards), each after a spacer, for the agent to append. Empty once
/// there are no more.
pub async fn more(Path(page): Path<usize>) -> Response {
    if page < 2 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let posts = news().posts.iter().skip((page - 1).saturating_mul(PAGE_SIZE)).take(PAGE_SIZE);
    let body: String = posts.map(|post| format!("{SPACER}{post}")).collect();
    log!("GET noticias, página {page}: {} bytes", body.len());
    html(body)
}

fn html(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], body).into_response()
}

/// Splits the page around its posts. A page without posts stays whole, as its head.
fn split(page: &str) -> News {
    let mut posts = Vec::new();
    let (mut first, mut last) = (None, 0);
    let mut from = 0;
    while let Some(start) = find_div(page, from, "<div class=\"post") {
        let Some(end) = block_end(page, start) else { break };
        first.get_or_insert(start);
        posts.push(page[start..end].to_string());
        (last, from) = (end, end);
    }
    let Some(first) = first else {
        return News { head: page.to_string(), posts, tail: String::new() };
    };
    let mut tail = page[last..].to_string();
    if let Some(start) = find_div(&tail, 0, "<div id=\"footer") {
        if let Some(end) = block_end(&tail, start) {
            tail.replace_range(start..end, "");
        }
    }
    News { head: page[..first].to_string(), posts, tail }
}

/// The next `<div` from `from` that starts with `opening`, a class or id that ends there or
/// goes on with another class: `<div class="post` matches `post` and `post text`, not `posts`.
fn find_div(html: &str, mut from: usize, opening: &str) -> Option<usize> {
    while let Some(start) = html[from..].find(opening).map(|i| from + i) {
        let after = start + opening.len();
        if matches!(html.as_bytes().get(after), Some(b'"' | b' ')) {
            return Some(start);
        }
        from = after;
    }
    None
}

/// The end of the `<div>` that starts at `start`, past its matching `</div>`.
fn block_end(html: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut at = start;
    loop {
        let close = html[at..].find("</div>").map(|i| at + i)?;
        match html[at..].find("<div").map(|i| at + i) {
            Some(open) if open < close => {
                depth += 1;
                at = open + "<div".len();
            }
            _ => {
                depth -= 1;
                at = close + "</div>".len();
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    }
}
