#!/usr/bin/env python3
"""Generate a throwaway Zorite database for performance testing and screenshots.

Builds a SQLite file at the **current** schema (v7) with a large synthetic graph:
a 3-level `Area::Topic::Note` namespace tree (plus flat pages), a couple weeks of
journal days, `[[wiki-links]]` (indexed into `page_links`), inline images, the
occasional Mermaid diagram, a set of freeform **whiteboards** (real scene JSON —
boxes, arrows, labels, shapes), a few **page aliases**, some **favorites**, and a
couple of **whiteboard templates**. Deterministic (fixed seed) so runs reproduce.

Because the file is written at v7 (matching `src/db.rs`'s `SCHEMA_VERSION`), the
app opens it with no migration. The earlier versions of this script wrote v3 and
relied on the app to migrate; whiteboards/favorites need the v6/v7 schema, so we
write it directly here.

Usage:
    python3 scripts/gen_perf_db.py [COUNT] [OUTPUT]
        COUNT   number of named pages to generate (default 10000)
        OUTPUT  database path (default <repo>/db/zorite-perf.db)

Point the app at it without touching your real notes (run from the repo root):
    launchctl setenv ZORITE_DATA "$PWD/db/data-10k" && open <Zorite.app>   # macOS GUI
    ZORITE_DB=db/zorite-perf.db cargo run                                  # CLI

Image references only render if files with these names exist in the data dir's
`images/` folder (the data dir is NOT affected by ZORITE_DB) — edit IMAGES to
match yours, or ignore (missing images simply don't render).
"""
import sqlite3, random, os, sys, datetime, time, json, math

COUNT = int(sys.argv[1]) if len(sys.argv) > 1 else 10000
# Default into the repo's gitignored db/ folder (alongside the active test DB),
# robust to the caller's CWD; an explicit OUTPUT arg still wins.
_REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PATH = sys.argv[2] if len(sys.argv) > 2 else os.path.join(_REPO, "db", "zorite-perf.db")
IMAGES = [
    # Image filenames present in <data_dir>/images/ to see them render. Override
    # without editing this file via ZORITE_PERF_IMAGES="a.jpg,b.jpg".
    "sample-1.jpg",
    "sample-2.jpg",
]
if os.environ.get("ZORITE_PERF_IMAGES"):
    IMAGES = [s.strip() for s in os.environ["ZORITE_PERF_IMAGES"].split(",") if s.strip()]
random.seed(42)

os.makedirs(os.path.dirname(PATH) or ".", exist_ok=True)  # db/ is gitignored, may not exist yet
if os.path.exists(PATH):
    os.remove(PATH)

t0 = time.time()
con = sqlite3.connect(PATH)
cur = con.cursor()
# Schema v7, mirroring src/db.rs (fresh-install + every migration, cumulatively).
# The FTS5 index + triggers are added after the bulk insert, lower down.
cur.executescript(
    """
    CREATE TABLE pages (
        id           INTEGER PRIMARY KEY,
        title        TEXT NOT NULL UNIQUE,
        is_journal   INTEGER NOT NULL DEFAULT 0,
        journal_date TEXT UNIQUE,
        content      TEXT NOT NULL DEFAULT '',
        created_at   TEXT NOT NULL DEFAULT (datetime('now')),
        updated_at   TEXT NOT NULL DEFAULT (datetime('now')),
        kind         TEXT NOT NULL DEFAULT 'page'
    );
    CREATE TABLE page_links (
        source_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
        target_page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
        PRIMARY KEY (source_page_id, target_page_id)
    );
    CREATE INDEX idx_page_links_target ON page_links(target_page_id);
    CREATE TABLE page_aliases (
        page_id INTEGER NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
        alias   TEXT NOT NULL,
        PRIMARY KEY (page_id, alias)
    );
    CREATE INDEX idx_page_aliases_alias ON page_aliases(alias COLLATE NOCASE);
    CREATE TABLE whiteboard_templates (
        id         INTEGER PRIMARY KEY,
        name       TEXT NOT NULL,
        content    TEXT NOT NULL,
        created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
    CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    PRAGMA user_version = 7;
    """
)

WORDS = ("lorem ipsum dolor sit amet consectetur adipiscing elit sed do eiusmod "
         "tempor incididunt ut labore et dolore magna aliqua enim ad minim veniam "
         "quis nostrud exercitation ullamco laboris nisi aliquip ex ea commodo").split()

def para(n):
    return " ".join(random.choice(WORDS) for _ in range(n)).capitalize() + "."

# A couple of Mermaid diagrams, dropped into a small fraction of pages so the
# renderer (and the lightbox) have something to show in screenshots.
MERMAIDS = [
    "```mermaid\nflowchart TD\n  A[Capture] --> B{Parsed?}\n"
    "  B -->|yes| C[Store]\n  B -->|no| D[Retry]\n  C --> E[Report]\n```",
    "```mermaid\nsequenceDiagram\n  Client->>API: request\n  API->>DB: query\n"
    "  DB-->>API: rows\n  API-->>Client: json\n```",
    "```mermaid\nflowchart LR\n  Commit --> CI --> Build --> Test --> Release\n```",
]

# --- whiteboard scene building --------------------------------------------
#
# Scenes are gpui_whiteboard::Scene JSON (see crates/gpui-whiteboard): a camera
# plus z-ordered elements. ElementKind is externally tagged snake_case, e.g.
# {"rect": {x,y,w,h,width,rotation}}, {"arrow": {x1,y1,x2,y2,width}},
# {"text": {x,y,content,size,rotation}}. Colors are packed 0xRRGGBBAA u32s;
# `stroke: None` (omitted) follows the theme ink so a board reads in light + dark.

def pack(r, g, b, a=255):
    return (r << 24) | (g << 16) | (b << 8) | a

# Translucent fills (alpha 0x33) tint a box without killing the theme-ink label's
# contrast on either a light or a dark canvas.
FILLS = [pack(0x3B, 0x82, 0xF6, 0x33), pack(0x10, 0xB9, 0x81, 0x33),
         pack(0xF5, 0x9E, 0x0B, 0x33), pack(0xEF, 0x44, 0x44, 0x33),
         pack(0x8B, 0x5C, 0xF6, 0x33), pack(0x14, 0xB8, 0xA6, 0x33)]

class Board:
    """Accumulates elements with unique ids, emits Scene / template JSON."""
    def __init__(self):
        self.els, self.n = [], 0

    def _id(self):
        self.n += 1
        return self.n

    def shape(self, kind, x, y, w, h, label=None, fill=None, stroke=None, width=2.0, size=20.0):
        e = {"id": self._id(), "kind": {kind: {"x": x, "y": y, "w": w, "h": h,
                                               "width": width, "rotation": 0.0}}}
        if stroke is not None:
            e["stroke"] = stroke
        if fill is not None:
            e["fill"] = fill
        self.els.append(e)
        if label:
            self.text(x + w / 2.0, y + h / 2.0, label, size=size, center=True)
        return (x, y, w, h)

    def box(self, *a, **k):
        return self.shape("rect", *a, **k)

    def text(self, x, y, content, size=20.0, center=False, stroke=None):
        if center:  # x,y is the desired center; store the top-left (estimated)
            x -= len(content) * size * 0.52 / 2.0
            y -= size * 1.2 / 2.0
        e = {"id": self._id(), "kind": {"text": {"x": x, "y": y, "content": content,
                                                "size": size, "rotation": 0.0}}}
        if stroke is not None:
            e["stroke"] = stroke
        self.els.append(e)

    def seg(self, kind, x1, y1, x2, y2, width=2.0, stroke=None):
        e = {"id": self._id(), "kind": {kind: {"x1": x1, "y1": y1, "x2": x2, "y2": y2,
                                              "width": width}}}
        if stroke is not None:
            e["stroke"] = stroke
        self.els.append(e)

    def connect(self, a, b, arrow=True):
        """An edge-to-edge connector between two boxes along their dominant axis."""
        ax, ay, aw, ah = a
        bx, by, bw, bh = b
        acx, acy, bcx, bcy = ax + aw / 2, ay + ah / 2, bx + bw / 2, by + bh / 2
        if abs(bcx - acx) >= abs(bcy - acy):
            if bcx >= acx:
                x1, y1, x2, y2 = ax + aw, acy, bx, bcy
            else:
                x1, y1, x2, y2 = ax, acy, bx + bw, bcy
        else:
            if bcy >= acy:
                x1, y1, x2, y2 = acx, ay + ah, bcx, by
            else:
                x1, y1, x2, y2 = acx, ay, bcx, by + bh
        self.seg("arrow" if arrow else "line", x1, y1, x2, y2)

    def freehand(self, points, width=3.0, stroke=None):
        e = {"id": self._id(), "kind": {"draw": {"points": points, "width": width}}}
        if stroke is not None:
            e["stroke"] = stroke
        self.els.append(e)

    def scene(self):
        return json.dumps({"camera": {"x": 0.0, "y": 0.0, "zoom": 1.0}, "elements": self.els})

    def elements_json(self):  # for whiteboard_templates (a bare Vec<Element>)
        return json.dumps(self.els)

# Hand-designed boards (title, scene). Realistic infra/dev diagrams that look
# right in a screenshot, in roughly the same flowchart idiom Logseq boards use.
def _board_network():
    b = Board()
    inet = b.box(440, 60, 180, 56, "Internet")
    fw = b.box(440, 180, 180, 56, "Firewall", fill=FILLS[3])
    core = b.box(440, 300, 180, 56, "Core Switch", fill=FILLS[0])
    a1 = b.box(230, 440, 180, 56, "Access SW 1")
    a2 = b.box(650, 440, 180, 56, "Access SW 2")
    for pair in [(inet, fw), (fw, core), (core, a1), (core, a2)]:
        b.connect(*pair)
    return ("Network Topology", b.scene())

def _board_auth():
    b = Board()
    c = b.box(80, 250, 170, 56, "Client")
    api = b.box(360, 250, 170, 56, "API Gateway", fill=FILLS[0])
    auth = b.box(640, 130, 170, 56, "Auth Service", fill=FILLS[4])
    db = b.box(640, 370, 170, 56, "Database", fill=FILLS[1])
    for pair in [(c, api), (api, auth), (api, db)]:
        b.connect(*pair)
    return ("Auth Flow", b.scene())

def _board_pipeline():
    b = Board()
    prev = None
    for i, name in enumerate(["Commit", "CI", "Build", "Test", "Release", "Deploy"]):
        box = b.box(80 + i * 200, 280, 150, 54, name, fill=FILLS[i % len(FILLS)])
        if prev:
            b.connect(prev, box)
        prev = box
    return ("Release Pipeline", b.scene())

def _board_runbook():
    b = Board()
    start = b.box(380, 60, 200, 54, "Alert fires")
    chk = b.shape("diamond", 360, 180, 240, 120, "Service up?", fill=FILLS[2])
    ok = b.box(120, 380, 200, 54, "Log + close")
    esc = b.box(640, 380, 200, 54, "Page on-call", fill=FILLS[3])
    b.connect(start, chk)
    b.connect(chk, ok)
    b.connect(chk, esc)
    return ("Incident Runbook", b.scene())

def _board_mindmap():
    b = Board()
    hub = b.shape("ellipse", 420, 250, 200, 80, "Project", fill=FILLS[4])
    leaves = ["Scope", "Risks", "Budget", "Team", "Timeline"]
    for i, name in enumerate(leaves):
        ang = (i / len(leaves)) * 2 * math.pi
        lx, ly = 520 + 320 * math.cos(ang) - 80, 290 + 220 * math.sin(ang) - 28
        leaf = b.shape("ellipse", lx, ly, 160, 56, name)
        b.connect(hub, leaf, arrow=False)
    return ("Project Mind Map", b.scene())

def _board_kanban():
    b = Board()
    for i, col in enumerate(["To do", "Doing", "Done"]):
        x = 80 + i * 280
        b.box(x, 60, 240, 520, None, fill=FILLS[i])
        b.text(x + 120, 90, col, size=22, center=True)
        for j in range(random.randint(2, 4)):
            b.box(x + 20, 130 + j * 90, 200, 64, para(2)[:18])
    return ("Sprint Board", b.scene())

def _board_sketch():
    b = Board()
    b.text(120, 80, "Ideas", size=28)
    b.freehand([[140 + k * 18, 200 + 30 * math.sin(k / 1.5)] for k in range(20)])
    proto = b.box(120, 320, 220, 70, "Prototype", fill=FILLS[5])
    fb = b.box(440, 320, 220, 70, "Feedback")
    b.connect(proto, fb)
    return ("Whiteboard Sketch", b.scene())

DESIGNED = [_board_network, _board_auth, _board_pipeline, _board_runbook,
            _board_mindmap, _board_kanban, _board_sketch]

def _board_procedural(i):
    """A small random flowchart so the Whiteboards list looks lived-in at scale."""
    b = Board()
    vertical = (i % 2 == 0)
    n = random.randint(3, 5)
    boxes = []
    for k in range(n):
        if vertical:
            x, y = 380, 80 + k * 130
        else:
            x, y = 100 + k * 230, 280
        fill = random.choice(FILLS) if random.random() < 0.5 else None
        boxes.append(b.box(x, y, 190, 56, para(2)[:16], fill=fill))
    for k in range(1, n):
        b.connect(boxes[k - 1], boxes[k])
    return (f"Diagram {i:02d}", b.scene())

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
    if random.random() < 0.04:  # a sprinkling of Mermaid diagrams
        parts += [random.choice(MERMAIDS), ""]
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

# Whiteboards: the hand-designed set, then procedural filler scaled to COUNT.
n_wb = min(60, max(len(DESIGNED) + 9, COUNT // 1200))
boards = [fn() for fn in DESIGNED]
for i in range(len(DESIGNED), n_wb):
    boards.append(_board_procedural(i))
wb_titles = [t for t, _ in boards]
cur.executemany("INSERT INTO pages (title, content, kind) VALUES (?, ?, 'whiteboard')", boards)
con.commit()

# Resolve ids for link edges, aliases, and favorites.
cur.execute("SELECT id, title FROM pages")
id_of = {title: pid for pid, title in cur.fetchall()}

edges = {(id_of[t], id_of[l]) for t, links in content_links.items()
         for l in links if l in id_of and id_of[l] != id_of[t]}
cur.executemany("INSERT OR IGNORE INTO page_links (source_page_id, target_page_id) VALUES (?, ?)",
                list(edges))

# A few page aliases (alternate names that resolve to a page).
aliases = []
for title in titles[: min(COUNT, 600)]:
    if random.random() < 0.05:
        aliases.append((id_of[title], "alt-" + title.split("::")[-1].lower()))
cur.executemany("INSERT OR IGNORE INTO page_aliases (page_id, alias) VALUES (?, ?)", aliases)

# Full-text search: external-content FTS5 over 'page' rows + the v6 triggers
# (kept verbatim from src/db.rs so the app behaves identically). Built after the
# bulk insert so we tokenize once rather than per-row.
cur.executescript(
    """
    CREATE VIRTUAL TABLE pages_fts USING fts5(
        title, content, content='pages', content_rowid='id', tokenize='trigram'
    );
    INSERT INTO pages_fts(rowid, title, content)
        SELECT id, title, content FROM pages WHERE kind = 'page';
    CREATE TRIGGER pages_fts_ai AFTER INSERT ON pages WHEN new.kind = 'page' BEGIN
        INSERT INTO pages_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
    END;
    CREATE TRIGGER pages_fts_ad AFTER DELETE ON pages WHEN old.kind = 'page' BEGIN
        INSERT INTO pages_fts(pages_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
    END;
    CREATE TRIGGER pages_fts_au AFTER UPDATE ON pages WHEN new.kind = 'page' BEGIN
        INSERT INTO pages_fts(pages_fts, rowid, title, content)
            VALUES ('delete', old.id, old.title, old.content);
        INSERT INTO pages_fts(rowid, title, content) VALUES (new.id, new.title, new.content);
    END;
    """
)

# Favorites (sidebar group): a few named pages + a couple of boards, stored as a
# comma-separated id list in settings — the same shape AppView::load_favorites reads.
fav_titles = ["Area00", "Area01::Topic00", "Area00::Topic00::Note00",
              wb_titles[0], wb_titles[3]]
fav_ids = [str(id_of[t]) for t in fav_titles if t in id_of]
cur.execute("INSERT OR REPLACE INTO settings (key, value) VALUES ('favorites', ?)",
            (",".join(fav_ids),))

# A couple of whiteboard templates (content = a bare Vec<Element>).
def _tmpl_step():
    b = Board()
    b.box(0, 0, 180, 60, "Step", fill=FILLS[0])
    return b.elements_json()

def _tmpl_decision():
    b = Board()
    b.shape("diamond", 0, 0, 200, 110, "Decision?", fill=FILLS[2])
    return b.elements_json()

cur.executemany("INSERT INTO whiteboard_templates (name, content) VALUES (?, ?)",
                [("Process step", _tmpl_step()), ("Decision", _tmpl_decision())])
con.commit()

def count(q):
    return cur.execute(q).fetchone()[0]

n_pages = count("SELECT COUNT(*) FROM pages WHERE is_journal=0 AND kind='page'")
n_journ = count("SELECT COUNT(*) FROM pages WHERE is_journal=1")
n_board = count("SELECT COUNT(*) FROM pages WHERE kind='whiteboard'")
n_ns = count("SELECT COUNT(*) FROM pages WHERE title LIKE '%::%' AND kind='page'")
n_links = count("SELECT COUNT(*) FROM page_links")
n_alias = count("SELECT COUNT(*) FROM page_aliases")
n_tmpl = count("SELECT COUNT(*) FROM whiteboard_templates")
con.close()
print(f"named pages : {n_pages}")
print(f"journal days: {n_journ}")
print(f"whiteboards : {n_board}")
print(f"templates   : {n_tmpl}")
print(f"namespaced  : {n_ns}")
print(f"with images : {img_count}")
print(f"aliases     : {n_alias}")
print(f"favorites   : {len(fav_ids)}")
print(f"links       : {n_links}")
print(f"db size     : {os.path.getsize(PATH)/1024/1024:.1f} MB")
print(f"gen time    : {time.time()-t0:.1f}s -> {PATH}")
