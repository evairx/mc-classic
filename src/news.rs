//! The launcher's news pane: the web in `assets/`, served from this exe. `index.html` is the
//! first page and `2.html`, `3.html`... the older ones, as on the Tumblr it stands in for. The
//! pane is a Swing `JEditorPane`, which renders simple HTML and runs no JavaScript, so the
//! infinite scroll lives in the agent (`NewsScroller.java`): near the bottom it appends
//! `page/<n>`, the posts of `<n>.html` alone, until a page comes back empty. The title and the
//! sidebar of `index.html` show only once, at the top.
//!
//! The posts are the `<div class="post ...">` blocks of a page. Whatever lies between them (the
//! spacers) is dropped and put back between the posts each page adds.

use std::path::Path as FilePath;

use axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

/// Every file under `assets/`, as (path relative to it, contents). See build.rs.
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/assets.rs"));

const FIRST_PAGE: &str = "index.html";
const SPACER: &str = "\n\n              <div class=\"spacer\"></div>\n\n              ";

/// The first page: `index.html` without its footer, whose links to the other pages the infinite
/// scroll replaces.
pub async fn first_page() -> Response {
    let Some(page) = text(FIRST_PAGE) else { return not_found(FIRST_PAGE) };
    log!("GET noticias: {FIRST_PAGE}");
    html(remove_block(page, "<div id=\"footer", "div"))
}

/// The posts of `<page>.html` (2 onwards), each after a spacer, for the agent to append. Empty
/// once there are no more pages.
pub async fn more(Path(page): Path<usize>) -> Response {
    if page < 2 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let posts = text(&format!("{page}.html")).map(posts).unwrap_or_default();
    log!("GET noticias, página {page}: {} posts", posts.len());
    html(posts.iter().map(|post| format!("{SPACER}{post}")).collect())
}

/// Any other file of `assets/`, such as `assets/bg_main.png`.
pub async fn file(Path(path): Path<String>) -> Response {
    let Some((name, bytes)) = FILES.iter().find(|(name, _)| *name == path) else { return not_found(&path) };
    log!("GET 200 /mcupdate.tumblr.com/{path} ({} bytes)", bytes.len());
    ([(header::CONTENT_TYPE, crate::content_type(FilePath::new(name)))], *bytes).into_response()
}

fn text(name: &str) -> Option<&'static str> {
    FILES.iter().find(|(file, _)| *file == name).and_then(|(_, bytes)| std::str::from_utf8(bytes).ok())
}

fn html(body: String) -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], without_link_borders(&body)).into_response()
}

/// Adds `border="0"` to every `<img>` that sets no border. JEditorPane ignores the stylesheet's
/// `img { border: 0 }` and draws a 2px border in the link colour around images inside links.
fn without_link_borders(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(start) = rest.find("<img") {
        let end = rest[start..].find('>').map_or(rest.len(), |i| start + i);
        out.push_str(&rest[..start + "<img".len()]);
        if !rest[start..end].contains("border") {
            out.push_str(" border=\"0\"");
        }
        rest = &rest[start + "<img".len()..];
    }
    out.push_str(rest);
    out
}

fn not_found(what: &str) -> Response {
    log!("GET 404 noticias: {what} no está en assets/");
    StatusCode::NOT_FOUND.into_response()
}

fn posts(page: &str) -> Vec<&str> {
    let mut posts = Vec::new();
    let mut from = 0;
    while let Some(start) = find_div(page, from, "<div class=\"post") {
        let Some(end) = block_end(page, start, "div") else { break };
        posts.push(&page[start..end]);
        from = end;
    }
    posts
}

/// `html` without the first `<tag>` block that starts with `opening`.
fn remove_block(html: &str, opening: &str, tag: &str) -> String {
    let mut html = html.to_string();
    if let Some(start) = find_div(&html, 0, opening) {
        if let Some(end) = block_end(&html, start, tag) {
            html.replace_range(start..end, "");
        }
    }
    html
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

/// The end of the `<tag>` element that starts at `start`, past its matching closing tag.
fn block_end(html: &str, start: usize, tag: &str) -> Option<usize> {
    let (open, close) = (format!("<{tag}"), format!("</{tag}>"));
    let mut depth = 0;
    let mut at = start;
    loop {
        let next_close = html[at..].find(&close).map(|i| at + i)?;
        match html[at..].find(&open).map(|i| at + i) {
            Some(next_open) if next_open < next_close => {
                depth += 1;
                at = next_open + open.len();
            }
            _ => {
                depth -= 1;
                at = next_close + close.len();
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    }
}
