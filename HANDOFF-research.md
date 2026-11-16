# Handoff: Research section (in progress)

Goal: a Research section in CocoaDesk, similar to News, that looks up a query in scholarly databases.
Agreed scope (all in one go): search, watched topics, in-app reader, library, citations, explore, tie-ins.
Russian papers are required (CyberLeninka). GOST is the default citation style. A Wikipedia overview card goes above the results. No AI summaries, as in News.

## Done (tested)

**Core, `crates/core/src/research/`** (25 unit tests pass: `cargo test -p cocoadesk-core research`)
- `mod.rs`: `Paper`, `Author` (parses "Ivan I. Petrov", "Петров И. И.", "Petrov IA"), `Query` with filters (years, open access, min citations, kind, language, sources, exclude words, blocked), merge/dedupe by DOI, arXiv id or title+year, sort (relevance interleave, newest, most cited), blocking by journal name or domain.
- `sources.rs`: OpenAlex, arXiv, Semantic Scholar, Europe PMC, CyberLeninka (`POST /api/search`). Also the Wikipedia overview (search, then summary, kept only if on topic), Unpaywall PDF lookup, `citation_*` meta enrichment, and explore (cited by / references / similar: Semantic Scholar first, OpenAlex fallback). Fixtures are in `testdata/`.
- `cite.rs`: ГОСТ Р 7.0.100–2018 (1–3 authors heading; 4 all listed; 5+ «[и др.]»; DOI, otherwise URL with the access date), APA 7, BibTeX.
- `store.rs`: settings (keys, style, default sources), papers upsert, topics CRUD and checks, hits, library (saved, read, note, subject, collections, task card), blocked list, cached full texts, views (`topics_view` with a weekly digest, `library_view` with shelves), `read_paper`, `fetch_pdf`, `add_to_tasks` ("Read: …" card in the first non-done list), pruning.
- Migration 6 is in `crates/core/src/store.rs`.
- `news/feed.rs`: the `Fetch` trait gained `get_api` (honest User-Agent, required by Wikipedia), `post_json` and `get_file` (large files).
- Live test: `cargo test -p cocoadesk-core live_research -- --ignored --nocapture`. It worked against CyberLeninka, Europe PMC and Semantic Scholar, and full text loaded from CyberLeninka pages.

**Shell, `src-tauri/src/lib.rs`** (compiles: `cargo check -p cocoadesk`)
- All `research_*` commands are registered.
- The `paperpdf://` protocol serves downloaded PDFs from `%APPDATA%/…/research-pdf/<id>.pdf`.
- A research timer runs every 60 s with hourly pruning, and checks send notifications.
- The CSP now has `frame-src paperpdf: http://paperpdf.localhost` for the PDF iframe.

**Frontend**
- `src/api.ts`: research types, `researchApi`, `paperPdfUrl`. Nothing uses them yet.

## Not done yet

1. `src/ui/research.ts`, a `ResearchScreen` with three tabs (Search, Watched, Library). Model it on `src/ui/news.ts`.
   - **Search:** query bar and filters, database chips, Wikipedia overview card, results, errors per database, an explore mode ("Papers citing «X»" with a Back button), and "Watch this search".
   - **Paper row:** title, authors, year, venue, chips (kind, Free, citations, sources), save bookmark, and a menu with: open page or PDF, copy GOST/APA/BibTeX, cited by / references / similar, Add to Tasks, link to a subject, block the journal or website, mark read.
   - **Reader pane** (like the News reader): Abstract / Full text / PDF tabs (iframe at `paperPdfUrl` after `researchApi.pdf`), Save PDF, Semantic Scholar's TLDR labelled as machine-written, keywords, a cite box with a style switch and Copy, notes (save on blur), collections chips, a subject select (`classSubjects`), Add to Tasks (board picker from `taskBoards`), and explore buttons.
   - **Watched:** a topic list like News, status line, Check now, Mark all read, Edit, a "This week" digest card, All/New/Saved.
   - **Library:** shelves (All, Unread, collections with "New collection", subjects) and "Copy reference list" (GOST, sorted: Russian first, then foreign).
2. Dialogs:
   - Topic dialog: query and filters, interval in hours or days (minimum 1 h), notify, enabled, delete.
   - Research settings: OpenAlex key with a link to openalex.org/settings/api, Semantic Scholar key, email for Unpaywall, default style, default databases.
   - Blocked journals and websites.
3. `src/main.ts`: add a `research` section under Monitor (new icon in `src/ui/icons.ts`) and listen to `research:changed` / `research:checking`.
4. CSS in `src/styles/app.css`: reuse the `.reader`, `.prose`, `.digest`, `.topics` and `.nstat` styles; add paper-row styles.
5. README section and `npm run typecheck`, then a branch and PR.

## Notes

- OpenAlex now needs a free API key (from Feb 2026; about $1/day free with a key, very little without). The UI should prompt for one when it sees the "allowance used up" error.
- Semantic Scholar often returns 429 without a key. That's handled as a non-fatal error per database.
- arXiv can be slow (25 s timeout).
- User preferences: no Claude attribution in commits or PRs; never use the middle-dot "·" as a divider in UI text.
