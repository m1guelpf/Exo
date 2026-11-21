//! The inbox rows Exo reads from its store after syncing gmail-mock's default mailbox, checked
//! against the text they show at 1440×900: participants, subjects and snippets cut to the row's
//! width, dates and separators.

use exo::{
    format::{
        dates::{self, Segment},
        row,
    },
    mailbox::{Chips, ThreadSummary},
};
use exo_store::ListId;

/// The row's middle column and from label at 1440px.
const CENTER_WIDTH: f64 = 687.34;
const FROM_LABEL_WIDTH: f64 = 185.8;

async fn rows() -> Vec<String> {
    let config = gmail_mock::Config::default();
    let now = config.now.clone();
    let server = gmail_mock::MockServer::start(config)
        .await
        .expect("mock starts");
    let dir = std::env::temp_dir().join(format!("exo-inbox-rows-{}", std::process::id()));
    let tokens = gmail::StaticToken(gmail_mock::DEV_ACCESS_TOKEN.into());
    let store = exo_store::Store::open(&dir, "maya@brightmoor.co", server.endpoints(), tokens)
        .await
        .expect("store opens");
    while store.sync().await.expect("sync").backfilling {}
    let page = store
        .list(&ListId::Inbox, None, 100)
        .await
        .expect("inbox reads");
    let chips = Chips::new(&store.labels().await.expect("labels read"));
    let _ = std::fs::remove_dir_all(&dir);
    let items = page
        .threads
        .into_iter()
        .map(|thread| ThreadSummary::new(thread, &ListId::Inbox, &chips))
        .map(|thread| (thread.received, thread))
        .collect();
    dates::segment(items, &now)
        .into_iter()
        .map(|segment| match segment {
            Segment::Separator(label) => format!("— {label}"),
            Segment::Item(thread) => {
                let subject = row::subject(&thread.subject, CENTER_WIDTH, &thread.labels);
                let snippet = row::snippet(
                    &thread.snippet,
                    &[],
                    CENTER_WIDTH,
                    &thread.labels,
                    &subject,
                    false,
                )
                .map(|(text, cut)| if cut { format!("{text}…") } else { text })
                .unwrap_or_default();
                let marker = if thread.drafts > 0 { "Draft to " } else { "" };
                let from = row::participants(
                    &thread.participants,
                    (!thread.recipients).then_some(FROM_LABEL_WIDTH),
                    &[],
                );
                format!(
                    "{marker}{from} | {} | {subject} | {snippet} | {}",
                    thread.labels.join(","),
                    dates::row(thread.received, &now)
                )
            }
        })
        .collect()
}

#[tokio::test]
async fn rows_cut_their_text_to_the_row_at_1440px() {
    let rows = rows().await;
    let expected = [
        "Tom Becker |  | Design crit moved to 3pm | Heads up: today's design crit moves to 3pm, Daniel needed the 1pm slot. Same room. | 8:31 AM",
        "Grace Liu |  | Invitation: Coffee with Grace @ Wed Nov 25, 2026 9:00am - 9:45am… | Coffee with Grace When Wed Nov 25, 2026 9:00am… | 8:10 AM",
        "Priya Raman |  | 1:1 agenda | Few things for today: - Backend hiring: I'd like to open a second req now rather than in January - Atlas rate-card… | 8:02 AM",
        "Parcelpoint |  | Out for delivery: your package from Field & Fell | Your package is out for delivery and should arrive by 8pm today. … | 7:50 AM",
        "Quorum |  | 6 new messages in #support | You have 6 unread messages in #support. Chloé Martin: \"Can someone take a look at the… | 7:16 AM",
        "Daniel Reyes |  | Board deck — draft 2 | Draft 2 attached. Maya, the product section still has placeholder numbers on slide 9… | 7:12 AM",
        "Morning Ledger |  | Tuesday, Nov 17: Why corporate travel is still below 2019 | Why corporate travel is still below 2019 Good morning. … | 6:30 AM",
        "Tracklight |  | [brightmoor/atlas] Build passed: main (9b1e044) | Build passed on main (9b1e044). All 214 checks passed in 6m 41s. … | 6:12 AM",
        "— Yesterday",
        "Zara, Tom | Hiring | Debrief: Senior Product Designer (Aiko) | Yes from me too. Some concern about how much she's worked directly… | NOV 16",
        "Rachel, Dev Malhotra | Clients/Calloway | Atlas beta: feedback from our dispatch team | Also, our VP saw the demo on Friday and asked whether… | NOV 16",
        "Daniel, Priya | Projects/Atlas Launch | Q4 planning doc — comments by Thursday? | Great, December it is unless Lena sees a problem on the… | NOV 16",
        // Gmail-web drafts only show in Drafts.
        "Grace Liu |  | Coffee next week? | Maya, I'm in New York Tuesday through Thursday next week. Would love to grab coffee and hear more… | NOV 16",
        "Theo, Jordan |  | Harborlight pilot — next steps | Cold-chain alerts are on the Q1 roadmap. Happy to join a call with Theo to walk through it. | NOV 16",
        "Ben, Daniel | Clients/Calloway | MSA v3 redlines | 3x on §7.2 works. I'd accept 60 days on §9.1 if they drop the SLA credits to our standard… | NOV 16",
        "Freight Signals | Newsletters | Freight Signals: Spot rates rise 4% as peak season arrives | Spot rates rise 4% as peak season arrives Spot rates… | NOV 16",
        "Jordan Ellis |  | Invitation: Calloway: Atlas feedback call @ Thu Nov 19, … | Calloway: Atlas feedback call When Thu Nov 19, 2026 2:00pm… | NOV 16",
        "Morning Ledger |  | Monday, Nov 16: Ad spending shifts again as streaming grows | Ad spending shifts again as streaming grows Good morning.… | NOV 16",
        "Harbor Bank |  | Large transaction alert | A payment of $3,395.00 to Westbrook Realty was made from your checking account ending in 4417.… | NOV 16",
        "— Last 7 days",
        "Jess Moreno |  | dinner thurs? | booked! table under Moreno. no laptops at dinner 😤 | NOV 15",
        "Ana Ruiz |  | On designing for people who work in a hurry | On designing for people who work in a hurry Dispatchers, nurses, … | NOV 15",
        "Ngozi, Emeka | Family | Thanksgiving 🦃 | Wonderful!! I'll pick you up from the station. Don't forget a warm coat, it's supposed to be cold. | NOV 14",
        "Tom .. Oliver, Lena, Priya, Daniel | Projects/Atlas Launch | Atlas launch checklist | Calendar invite for go/no-go is out. 🚀 | NOV 13",
        "Sam .. Lena, Oliver, Daniel, Inês |  | Team offsite: Hudson, Dec 9–11 🏔️ | Millbrook Inn, 28 Warren St, Hudson NY. Ines, thank you! I'll set up a carpool sheet. | NOV 12",
        "— Earlier this month",
        "Mei Watanabe |  | Speaking at Hollis Conference 2027 | Wonderful! Abstracts are due January 10. The speaker guide is attached. … | NOV 9",
    ];
    assert_eq!(rows, expected);
}
