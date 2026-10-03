//! The page the decline address answers with, in the words the maintainer approved.

use crate::declining::Standing;

/// The page's title, in every state.
const TITLE: &str = "Decline an invitation";

/// What the page says, for a standing, or for a request over the rate limit.
pub(crate) enum Said<'a> {
    /// Where the invitation stands.
    Standing(&'a Standing),
    /// Too many requests from one address.
    Limited,
}

/// The whole page for what is said, with `token`'s form where it is still open.
pub(crate) fn page(said: &Said<'_>, token: &str) -> String {
    let body = match said {
        Said::Standing(Standing::Open(name)) => format!(
            "<p>You were invited to watch on this household's media server as {}.</p>\n\
             <form method=\"post\" action=\"/decline/{}\">\
             <button type=\"submit\">Decline the invitation</button></form>\n\
             <p class=\"small\">Declining closes this invitation and the account made for it. \
             Whoever invited you will see that you declined.</p>",
            escaped(name),
            escaped(token)
        ),
        Said::Standing(Standing::Declined) => paragraph(
            "You declined the invitation. The account made for you can no longer be signed in to.",
        ),
        Said::Standing(Standing::AlreadyDeclined) => {
            paragraph("This invitation was already declined.")
        }
        Said::Standing(Standing::NoLongerOpen) => paragraph("This invitation is no longer open."),
        Said::Standing(Standing::Accepted) => paragraph(
            "This invitation was accepted, so it cannot be declined here. \
             Ask whoever invited you to remove the account.",
        ),
        Said::Standing(Standing::Silent) => paragraph(
            "The media server did not answer, so nothing was declined. \
             Try again in a little while.",
        ),
        Said::Standing(Standing::Refused) => paragraph("This invitation cannot be declined here."),
        Said::Limited => paragraph("Try again in a minute."),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n\
         <title>{TITLE}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<main>\n\
         <h1>{TITLE}</h1>\n{body}\n</main>\n</body>\n</html>\n"
    )
}

/// One paragraph of text this service wrote.
fn paragraph(text: &str) -> String {
    format!("<p>{text}</p>")
}

/// `text` with every character HTML gives a meaning to written as an entity.
fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// The page's look: readable on a phone, in either colour scheme, with nothing fetched.
const STYLE: &str = "\
:root{color-scheme:light dark;font-family:system-ui,sans-serif;line-height:1.5}\
body{margin:0;padding:2rem 1rem}main{max-width:32rem;margin:0 auto}\
h1{font-size:1.5rem}button{font:inherit;padding:.6rem 1.2rem;border-radius:.4rem;\
border:1px solid currentColor;background:none;color:inherit;cursor:pointer}\
.small{font-size:.9rem;opacity:.8}";

#[cfg(test)]
mod tests;
