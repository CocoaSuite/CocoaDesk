# CocoaDesk

CocoaDesk is the personal desk of the Cocoa suite: one place to organise, keep an eye on and automate everyday digital life. It runs locally on Windows and keeps your data on your computer.

## What's in it so far

### Classes

- A week timetable laid out by periods, in the CocoaAuthor look.
- **Top and bottom weeks.** Each semester has a start date and says whether its first week is a top or a bottom week; weeks alternate from there. Holidays and breaks don't change the count.
- Classes can run every week, only in top weeks or only in bottom weeks, and can be limited to part of the semester (for example "November–December").
- **Marked done automatically** when they end. If the computer was asleep or off, CocoaDesk catches up when it's back. You can change any session to *missed* or *cancelled*, and cancelled classes are never marked or reminded.
- Classes added in the middle of a semester are only tracked from that moment on.
- **Reminders** a few minutes before each class, as Windows notifications.
- **Excel import** of university timetables: pick your group, see every class it found and untick the ones that aren't yours. It reads alternating weeks (`top / bottom`), merged lecture cells, teachers, rooms, remarks and month ranges, and takes the bell schedule from the file. Importing a newer version of the file updates classes in place and remembers what you unticked.
- Keeps running in the notification area when the window is closed, so marking and reminders carry on (can be turned off).

### News

- **Topics** you choose, each checked on its own schedule (every few minutes up to once a week), optionally only between set hours.
- A topic looks in **news searches** (Google News or Bing News, Russian or English edition) and any **RSS/Atom feeds** you add — paste a website and CocoaDesk finds its feed.
- Only stories that **mention your words** are kept. Word endings don't matter («электромобиль» also finds «электромобилей»); `word*` matches the start of a word and `"quotes"` match exactly. Stories mentioning words you want to leave out are dropped.
- **Block websites** for one topic or for every topic, including from any story's menu. Subdomains are blocked too, and stories already found from a blocked site are removed (saved ones stay).
- The same story from several websites is shown once, with the others underneath.
- **Read inside CocoaDesk**: click a story to read its text in a side panel, without ads and menus (the same method as browsers' reader view). Links inside open in your browser, and *Open in browser* is always there for videos, comments or pages that can't be read this way. Google News links are followed to the original article.
- **Today so far**: a short paragraph at the top of each topic, put together from counts — how many stories and websites today, the stories covered by the most websites (with the article's opening sentence once it has been read), and names that recur in today's headlines. Nothing is written or summarised by a model.
- Unread and saved stories. Unsaved stories are cleared after 30 days.
- Optional notifications for every new story, or only when a headline mentions certain words.

Matching is plain word comparison with a standard stemmer — no recommendations or ranking; stories are listed newest first.

## Development

Requires Node.js 22 and a stable Rust toolchain. On Windows, WebView2 is already part of the system.

```sh
npm install
npm run desktop     # run the app with live reload
npm run dist        # build the Windows installer into target/release/bundle/nsis
cargo test -p cocoadesk-core
```

The project is split into:

- `crates/core` — calendar rules, news matching and feeds, storage (SQLite), the timetable import and the background timer logic. Plain Rust with unit tests; no UI.
- `src-tauri` — the desktop shell: window, notification-area icon, timer threads (classes and news) and the commands the interface calls.
- `src` — the interface (TypeScript, no framework), styled after CocoaAuthor.

Data lives in `%APPDATA%\com.cocoasuite.cocoadesk\cocoadesk.db`.

To check the importer against a real timetable without adding it to the repository:

```sh
COCOADESK_SAMPLE_XLSX=path/to/file.xlsx COCOADESK_SAMPLE_GROUP="GROUP-01-26" cargo test -p cocoadesk-core reads_a_real_file -- --ignored --nocapture
```

## Builds

Every push to `main` builds a Windows installer; download it from the run's artifacts on the Actions tab. Pushing a tag such as `v0.1.0` also attaches the installer to a GitHub release.
