#!/usr/bin/env python3
"""Generate a throwaway zorite database for performance testing.

Creates a SQLite file with the v3 schema and a large set of synthetic pages:
a 3-level `Area::Topic::Note` namespace tree (plus a few flat pages), a couple
weeks of journal days, `[[wiki-links]]` (indexed into `page_links`), and inline
image references. Deterministic (fixed seed) so runs are reproducible.

Usage:
    python3 scripts/gen_perf_db.py [COUNT] [OUTPUT]
        COUNT   number of named pages to generate (default 10000)
        OUTPUT  database path (default /tmp/zorite-perf.db)

Point the app at it without touching your real notes:
    launchctl setenv ZORITE_DB /tmp/zorite-perf.db && open <zorite.app>   # macOS GUI
    ZORITE_DB=/tmp/zorite-perf.db cargo run                               # CLI

Image references only render if files with these names exist in the data dir's
`images/` folder (the data dir is NOT affected by ZORITE_DB) — edit IMAGES to
match yours, or ignore (missing images simply don't render).
"""
import sqlite3, random, os, sys, datetime, time

COUNT = int(sys.argv[1]) if len(sys.argv) > 1 else 10000
PATH = sys.argv[2] if len(sys.argv) > 2 else "/tmp/zorite-perf.db"
IMAGES = [
    # Replace with image filenames present in <data_dir>/images/ to see them render.
    "sample-1.jpg",
    "sample-2.jpg",
]
random.seed(42)

if os.path.exists(PATH):
    os.remove(PATH)

t0 = time.time()
con = sqlite3.connect(PATH)
cur = con.cursor()
cur.executescript(
    """
    CREATE TABLE pages (
        id           INTEGER PRIMARY KEY,
        title        TEXT NOT NULL UNIQUE,
        is_journal   INTEGER NOT NULL DEFAULT 0,
        journal_date TEXT UNIQUE,
        content      TEXT NOT NULL DEFAULT '',
        created_at   TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE page_links (
        source_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
        target_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
        PRIMARY KEY (source_page_id, target_page_id)
    );
    CREATE INDEX idx_page_links_target ON page_links(target_page_id);
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    PRAGMA user_version = 3;
    """
)

WORDS = ("lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod "
         "tempor incididunt ut labore et dolore magna aliqua enim ad minim veniam "
         "quis nostrud exercitation ullamco laboris nisi aliquip ex ea commodo").split()

def para(n):
    return " ".join(random.choice(WORDS) for _ in range(n)).capitalize() + "."

# 3-level namespace tree, padded with flat pages up to COUNT.
titles = []
a = 0
while len(titles) < COUNT:
    area = f"Area{a:02d}"
    titles.append(area)
    for t in range(10):
        topic = f"{area}::Topic{t:02d}"
        titles.append(topic)
        for n in range(23):
            titles.append(f"{topic}::Note{n:02d}")
    a += 1
# top up with flat pages if the tree under/overshot, then trim.
i = 0
while len(titles) < COUNT:
    titles.append(f"Page{i:05d}")
    i += 1
titles = titles[:COUNT]

def make_content(big, with_img):
    parts = ["# " + para(2), ""]
    for _ in range(random.randint(8, 20) if big else random.randint(1, 3)):
        parts += [para(random.randint(25, 70)), ""]
    if random.random() < 0.7:
        parts.append("## Items")
        parts += [f"- {para(random.randint(3, 8))}" for _ in range(random.randint(3, 8))]
        parts.append("")
    if with_img and IMAGES:
        parts += [f"![sample](images/{random.choice(IMAGES)}){{width={random.choice([200,300,374,450])}}}", ""]
    links = random.sample(titles, random.randint(1, 5))
    parts.append("See also: " + ", ".join(f"[[{l}]]" for l in links))
    parts.append(f"Tagged #area{random.randint(0, 39)} #topic{random.randint(0, 40)}")
    if random.random() < 0.3:
        parts += ["", "```rust", "fn demo() -> usize {", "    (0..100).sum()", "}", "```"]
    if random.random() < 0.2:
        parts += ["", "| Col A | Col B | Col C |", "|---|---|---|", "| 1 | 2 | 3 |", "| 4 | 5 | 6 |"]
    return "\n".join(parts), links

content_links, rows, img_count = {}, [], 0
for idx, title in enumerate(titles):
    with_img = random.random() < 0.12
    img_count += with_img
    body, links = make_content(idx % 20 == 0, with_img)
    content_links[title] = links
    rows.append((title, body))
cur.executemany("INSERT INTO pages (title, content) VALUES (?, ?)", rows)

today = datetime.date.today()
for d in range(14):
    day = (today - datetime.timedelta(days=d)).isoformat()
    links = random.sample(titles, 4)
    body = [f"## {day}", para(40), "", "Worked on " + ", ".join(f"[[{l}]]" for l in links), ""]
    if d % 3 == 0 and IMAGES:
        body += [f"![shot](images/{random.choice(IMAGES)}){{width=320}}", ""]
    cur.execute("INSERT INTO pages (title, is_journal, journal_date, content) VALUES (?, 1, ?, ?)",
                (day, day, "\n".join(body)))
con.commit()

cur.execute("SELECT id, title FROM pages")
id_of = {title: pid for pid, title in cur.fetchall()}
edges = {(id_of[t], id_of[l]) for t, links in content_links.items()
         for l in links if l in id_of and id_of[l] != id_of[t]}
cur.executemany("INSERT OR IGNORE INTO page_links (source_page_id, target_page_id) VALUES (?, ?)", list(edges))
con.commit()

def count(q):
    return cur.execute(q).fetchone()[0]

n_pages = count("SELECT COUNT(*) FROM pages WHERE is_journal=0")
n_journ = count("SELECT COUNT(*) FROM pages WHERE is_journal=1")
n_ns = count("SELECT COUNT(*) FROM pages WHERE title LIKE '%::%'")
n_links = count("SELECT COUNT(*) FROM page_links")
con.close()
print(f"named pages : {n_pages}")
print(f"journal days: {n_journ}")
print(f"namespaced  : {n_ns}")
print(f"with images : {img_count}")
print(f"links       : {n_links}")
print(f"db size     : {os.path.getsize(PATH)/1024/1024:.1f} MB")
print(f"gen time    : {time.time()-t0:.1f}s -> {PATH}")
