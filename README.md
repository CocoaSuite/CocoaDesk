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

### Tasks

Boards of lists and cards, in the spirit of Trello.

- **Several boards**, each with its own lists. New boards start with *To do*, *In progress*, *Review* and *Done*; add, rename, reorder or delete lists from a list's name.
- **Cards** have a title, labels (coloured bars, named per board), a due date, a description and a checklist. Add one quickly with the **+** on a list, or open a card for everything.
- **Drag cards** between and within lists. A card lifts and tilts as you swing it, the others slide aside to make room, and it settles into place with a small bounce. A quiet click plays when it lifts off and when it lands, a little different every time (pitch, brightness, loudness and left/right placement). Esc puts the card back. Sounds and their volume are in Board settings.
- **Salary (optional).** Enter what the client pays in dollars or rubles. The service's commission (10% by default, set per board, or per card when a job is different) is taken off, and the card shows what you actually get in both currencies. The dollar rate is the Central Bank of Russia's, updated daily, or one you type in yourself. A card marked **paid** keeps the rate of that day.
- **Monthly earnings**: what you've earned this month so far across all boards, what's still waiting on the board, and the last six months with totals and a monthly average. A card counts in the month it reached a list that *counts as done*. Figures update live while you drag a card into or out of Done.

### Moodboards

Boards of reference pictures in the spirit of PureRef, with Figma-like sections.

- **Several boards**, each an endless canvas. Switch, rename, duplicate or delete them from the board menu in the toolbar.
- **Add pictures** by pasting (screenshots, copied images, copied web pictures or image links), dropping files from Explorer, dragging pictures straight out of a browser, or *Add images*. Pictures are copied into CocoaDesk, so a board keeps working if the originals move; the same picture is stored once however often it's used, and pictures no board uses are cleared on the next start.
- **Arrange**: move, resize from the corners (pictures keep their shape; Shift stretches), rotate with the round handle (Shift snaps to 15°), Alt-drag for a copy, bring to front / send to back, *Original size*. Several items scale and rotate together.
- **Sections**: draw one with the section tool or wrap the selection (Ctrl+G). Anything inside a section moves with it; drag a section by its name and double-click the name to rename it.
- **Text** notes and **sketching**: pen, arrows, rectangles and ellipses in nine colours and three thicknesses, plus an eraser. Changing the colour or thickness also restyles what's selected.
- **Pin on top** turns the window into a small panel that stays above other apps, for drawing or painting next to your references. *Unpin* puts the window back.
- The wheel zooms at the pointer; Space-drag or the middle button pans; Shift+1 fits everything. Undo and redo cover every change, and the board saves itself as you go. The **?** button lists every shortcut.

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

- `crates/core` — calendar rules, news matching and feeds, storage (SQLite), the timetable import, moodboard storage and its picture files, task boards with salaries and the exchange rate, and the background timer logic. Plain Rust with unit tests; no UI.
- `src-tauri` — the desktop shell: window, notification-area icon, timer threads (classes and news), the `boardimg:` protocol that serves moodboard pictures, and the commands the interface calls.
- `src` — the interface (TypeScript, no framework), styled after CocoaAuthor.

Data lives in `%APPDATA%\com.cocoasuite.cocoadesk\cocoadesk.db`; moodboard pictures sit next to it in `board-images`.

To check the importer against a real timetable without adding it to the repository:

```sh
COCOADESK_SAMPLE_XLSX=path/to/file.xlsx COCOADESK_SAMPLE_GROUP="GROUP-01-26" cargo test -p cocoadesk-core reads_a_real_file -- --ignored --nocapture
```

## Builds

Every push to `main` builds a Windows installer; download it from the run's artifacts on the Actions tab. Pushing a tag such as `v0.1.0` also attaches the installer to a GitHub release.
