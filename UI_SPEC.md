# Murmur UI spec

Murmur's own UI design: layout, navigation, geometry, states, interactions and visual language.
The logo, illustrations and copy are Murmur's own. Fonts are Figtree and EB Garamond, both
licensed under the SIL Open Font License 1.1; the files and license texts live in `assets/fonts`
(`Figtree-OFL.txt`, `EBGaramond-OFL.txt`).

## Tokens (light theme)
| Token | Value | Use |
|---|---|---|
| `bg` | `#F5F4F0` | window, sidebar, stats card |
| `surface` | `#FCFCFC` | main content card |
| `surface_sunken` | `#F1F0EC` | grouped settings rows, chips |
| `nav_active` | `#EFEBE3` | active sidebar item pill |
| `border` | `#E8E6E1` | 1 px card borders, row dividers |
| `text` | `#1A1A1A` | primary text |
| `text_muted` | `#6B6A66` | secondary text, times, captions |
| `text_faint` | `#9A9893` | uppercase section labels |
| `primary_btn` | `#141414` bg / white text | "Add new", "Import", toggles on (`#1B1B1B`) |
| `secondary_btn` | `#ECEAE5` bg / text | "Change", "Show me how" |
| `toggle_off` | `#DDDCD8` | |
| `accent` | `#7C5CD6` (violet) | selected style card border, highlights, links |
| `danger` | `#D64545` | errors |
| `overlay` | black @ 28% | modal backdrop |
Dark theme: invert to `bg #161615`, `surface #1E1E1D`, `surface_sunken #262624`, `nav_active #2C2B29`, `border #33322F`, `text #F2F1EE`, `text_muted #A3A19C`; primary button becomes white bg/black text.

## Typography
- UI: **Figtree** (Regular 400, Medium 500, SemiBold 600). Base 14 px; nav 15 px medium; captions 12–13 px.
- Display: **EB Garamond** (Regular) for page titles in Settings ("General", "System"), big stats numbers ("13.1K"), and hero banner headlines. Page titles in the hub ("Dictionary", "Snippets", "Welcome back, …") are Figtree SemiBold 24 px.
- Section labels: 11 px, uppercase, letter-spaced (~0.08em), `text_faint` ("OCTOBER 21, 2026", "SETTINGS", "ACCOUNT").

## Window chrome (1350×850 default, min ~1000×640)
- Traffic lights over the sidebar (full-size content, hidden title). Top bar row height ~52 px: sidebar-toggle icon right of traffic lights (x≈108); notifications bell + account icons at top right.
- **Sidebar** 216 px wide on `bg`, no border. Logo row at y≈80: Murmur mark (simple waveform bars glyph) + wordmark "Murmur" 20 px semibold.
- Nav items: 192×36, radius 8, icon (18 px, stroke style) + label 15 px, 12 px left padding, 4 px gap between items. Active = `nav_active` pill; hover = `nav_active` at 60%.
- Top group: Home, Dictionary, Snippets, Style, (Notes later). Bottom group pinned to bottom: Settings, Help.
- **Content card**: `surface`, radius 16, 1 px `border`, inset 12 px from window right/bottom edges and 52 px from top; content max-width ~770 px centered with 40 px top padding.

## Pages
- **Home** ("Welcome back" + name from `NSFullUserName` first name):
  - Optional hero banner (rounded 16, gradient instead of photo; serif headline + one line + secondary button; dismissible ✕) — one tip, e.g. "Speak naturally — Murmur handles the punctuation."
  - Right column 250 px stats card on `bg`: three big serif numbers + small sans units ("13.1K total words", "99 wpm", "1 day streak"), divider, then a small section (e.g. model/cleanup status).
  - History: per-day uppercase date header with search icon at right; rows in one rounded bordered card, dividers between rows; row = time (muted, 80 px column, e.g. "10:47pm") + text. Hover row reveals icon buttons at right: copy, (play omitted), ⋯ menu (Delete). Empty state centered.
- **Dictionary / Snippets**: title left, buttons right ("Import" secondary-dark, "Add new" primary). Tabs underline style ("All", "Learned" for dictionary; "All" for snippets) with right-aligned icon buttons (search, sort). Dismissible hero card (gradient, serif headline with italic word, one sentence, chips: "Add new word" + example terms). List: rounded bordered card, rows 52 px with dividers; dictionary rows show word, ✨ marker for auto-learned (tooltip "Learned from your correction of …"), correction rules as "wrong → right"; snippets as "trigger → expansion" (expansion truncated). Row hover: edit/delete icons. "Add new" opens a centered modal (title, fields, Cancel/Save).
- **Style**: tabs "Personal messages | Work messages | Email | Other"; banner card describing which apps it applies to (gradient + a row of generic app-category icons, no real app logos); three tone cards side by side (title "Formal." / "Casual" / "very casual", subtitle "Caps + Punctuation" etc., example message bubble on `surface_sunken`, avatar circle at bottom right); selected card = 1.5 px `accent` border.
- **Settings**: a **modal** over the hub (backdrop `overlay`), 960×640, radius 16. Left nav 200 px on `bg` with uppercase labels "SETTINGS" (General, System, AI cleanup, Privacy) and "ACCOUNT" (Accounts & keys); version at bottom-left ("Murmur v0.1.0"). Right pane on `surface`: serif title 28 px, then grouped rows in `surface_sunken` cards radius 12; row = title (14 px medium) + caption (13 px muted) on the left, control on the right (secondary "Change" button 100×32, dropdown, or iOS-style toggle 36×20 black/grey). Section headers 14 px semibold above groups ("App settings", "Sound"). Esc / click backdrop closes.
  - General: Shortcuts (Hold fn and speak · Change), Microphone (· Change → dropdown), Language (dropdown).
  - System: App settings (Launch at login*, Show floating bar at all times, Show app in dock*), Sound (Dictation sounds), *show disabled with "coming soon" if not implemented.
  - AI cleanup: local model, cloud fallback + provider picker + credentials, time limit, learn from corrections, smart formatting. Cloud-issue banner at top.
  - Privacy: store history, clear history, permissions status rows with "Open settings".

## Dictation bar (pill) — bottom dock, centered, ~10 px above the Dock/bottom edge
| State | Size (W×H) | Look |
|---|---|---|
| resting | 40×8 | translucent dark sliver (black @ 55%), radius 6, 1 px white @ 50% border; hidden if "show at all times" off |
| hover / ready | 50×30 | expanded pill, radius 999 |
| listening (PTT) | 73×30 | black @ 85% pill, white waveform bars (≈9 bars, 2 px wide, 2 px gap, height from levels) |
| hands-free | ~102×30 | ✕ (cancel) left, waveform center, red ■ stop right |
| processing | 98×30 | three animated dots / shimmer |
| error | auto×30 | 1.5 px red border, short message |
| toast | auto×30 | text pill (existing toast feature) |
Transitions are 100 ms ease-out on width/height/opacity. Pill sits inside the fixed transparent overlay window.
