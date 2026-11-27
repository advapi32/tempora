# Task: categorize Windows apps

You are labelling Windows software packages for a screen-time tracker. The tracker shows how long someone spends in each app's window, grouped by category. For each package below, choose the ONE category that best describes what a person is doing while that app's window is in front of them.

## Categories (use these exact words)

- **Games**: games of any kind, and emulators.
- **Browsers**: general-purpose web browsers only (Chrome, Firefox, Edge, Brave, Vivaldi, Zen and similar).
- **Communication**: chat, voice and video calls, email, social networks and their clients, forum readers.
- **Media**: watching, listening and reading: video and music players, streaming services, podcast apps, e-book and comic readers, image viewers, RSS readers.
- **Productivity**: office suites, documents, PDF tools, notes, to-do and planning, calendars, mind maps, finance and accounting, translation, study tools, and professional or scientific desktop tools: GIS and mapping, statistics and data analysis, engineering simulation, bioinformatics, chemistry, medical and lab software.
- **Development**: IDEs, code and text editors, terminals and shells, SSH clients, Git tools, database and API clients, virtual machines and containers, reverse-engineering tools.
- **Creative**: making media: photo and image editing, drawing, vector design, 3D and CAD, video editing, audio editing and music production (DAWs), screen recording and live-streaming software.
- **Utilities**: system and maintenance tools: file managers, archivers, disk and cleanup tools, system monitors, launchers and search bars, clipboard and screenshot tools, password managers, VPNs, antivirus, remote desktop, backup and sync, hardware and peripheral control panels, mod managers.
- **Other**: game launchers and stores (Steam, Epic Games Launcher, GOG Galaxy, Battle.net, EA app, Ubisoft Connect), runtimes, SDKs, libraries, drivers, fonts, and anything that fits none of the above.
- **Unknown**: you can't tell what the app does from the row and don't recognize it with confidence.

## Rules

1. Decide by the app's main purpose, not by every feature it has.
2. Don't decide from the publisher alone: Microsoft, Google and Adobe make apps in almost every category.
3. Command-line tools: Development if aimed at developers, otherwise Utilities.
4. A row with only a name: use your own knowledge only if you're confident what that exact app is. Otherwise answer Unknown. A wrong guess is worse than Unknown.
5. Tags and descriptions may be in any language, or be marketing text. Read them for what the app actually does.
6. Use only the ten category words above, spelled exactly as written. Never invent, combine or qualify a category (not "Server Utilities", not "Games/Media").

## Input

A tab-separated table with columns: id, name, publisher, description, tags.

## Output

Return exactly one line per input row, in the same order, inside a single ```tsv code block:

```
<id><TAB><category>
```

- Copy the id exactly as given.
- No header line, no extra columns, no commentary before or after the code block.
- Every input row must appear once.
