use super::{page, Said};
use crate::declining::Standing;

fn said(standing: &Standing) -> String {
    page(&Said::Standing(standing), "token")
}

#[test]
fn every_state_carries_the_title() {
    for standing in [
        Standing::Open("Ana".to_owned()),
        Standing::Declined,
        Standing::AlreadyDeclined,
        Standing::NoLongerOpen,
        Standing::Accepted,
        Standing::Silent,
        Standing::Refused,
    ] {
        let shown = said(&standing);
        assert!(shown.contains("<title>Decline an invitation</title>"));
        assert!(shown.contains("<h1>Decline an invitation</h1>"));
    }
    assert!(page(&Said::Limited, "token").contains("<h1>Decline an invitation</h1>"));
}

#[test]
fn an_open_invitation_names_the_account_and_offers_the_refusal() {
    let shown = said(&Standing::Open("Ana".to_owned()));

    assert!(shown.contains("You were invited to watch on this household's media server as Ana."));
    assert!(shown.contains("<form method=\"post\" action=\"/decline/token\">"));
    assert!(shown.contains("Decline the invitation</button>"));
    assert!(shown.contains(
        "Declining closes this invitation and the account made for it. \
         Whoever invited you will see that you declined."
    ));
}

#[test]
fn each_other_state_says_the_approved_sentence_and_offers_nothing() {
    for (standing, sentence) in [
        (
            Standing::Declined,
            "You declined the invitation. The account made for you can no longer be signed in to.",
        ),
        (Standing::AlreadyDeclined, "This invitation was already declined."),
        (Standing::NoLongerOpen, "This invitation is no longer open."),
        (
            Standing::Accepted,
            "This invitation was accepted, so it cannot be declined here. \
             Ask whoever invited you to remove the account.",
        ),
        (
            Standing::Silent,
            "The media server did not answer, so nothing was declined. Try again in a little while.",
        ),
        (Standing::Refused, "This invitation cannot be declined here."),
    ] {
        let shown = said(&standing);
        assert!(shown.contains(sentence), "{sentence}");
        assert!(!shown.contains("<form"));
    }
    assert!(page(&Said::Limited, "token").contains("Try again in a minute."));
}

#[test]
fn a_name_and_a_token_are_escaped() {
    let shown = page(
        &Said::Standing(&Standing::Open("<b>\"A&o'\"</b>".to_owned())),
        "x\"><a",
    );

    assert!(shown.contains("as &lt;b&gt;&quot;A&amp;o&#39;&quot;&lt;/b&gt;."));
    assert!(shown.contains("action=\"/decline/x&quot;&gt;&lt;a\""));
}
