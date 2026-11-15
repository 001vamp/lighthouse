#!/usr/bin/env python3
"""Builds ROADMAP.html from ROADMAP.md, so the page never drifts from the file.

Run after every roadmap change:  python3 tools/roadmap_page.py
"""
import html
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC, OUT = ROOT / "ROADMAP.md", ROOT / "ROADMAP.html"


def inline(text):
    """Markdown inline syntax (code, bold, links) to HTML."""
    parts = re.split(r"(`[^`]+`)", text)
    out = []
    for part in parts:
        if part.startswith("`") and part.endswith("`") and len(part) > 1:
            out.append(f"<code>{html.escape(part[1:-1])}</code>")
            continue
        p = html.escape(part, quote=False)
        p = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", p)
        p = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", r'<a href="\2">\1</a>', p)
        out.append(p)
    return "".join(out)


def sections(text, level):
    """Splits text into {heading: body} at headings of the given level."""
    marker = "#" * level + " "
    result, current, lines = {}, None, []
    for line in text.split("\n"):
        if line.startswith(marker):
            if current is not None:
                result[current] = "\n".join(lines).strip()
            current, lines = line[len(marker):].strip(), []
        else:
            lines.append(line)
    if current is not None:
        result[current] = "\n".join(lines).strip()
    return result


def table(body):
    """Rows of the first markdown table in body, header first."""
    rows = []
    for line in body.split("\n"):
        line = line.strip()
        if not line.startswith("|"):
            if rows:
                break
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if all(re.fullmatch(r":?-+:?", c) for c in cells):
            continue
        rows.append(cells)
    return rows


def status_class(status):
    s = re.sub(r"[*]", "", status).lower()
    for key, cls in (("do this next", "next"), ("in progress", "now"), ("waiting", "wait"),
                     ("ready", "ready"), ("done", "done"), ("dropped", "dropped")):
        if s.startswith(key):
            return cls
    return "wait"


def status_word(status):
    return re.sub(r"[*]", "", status).split("·")[0].split(":")[0].strip()


def bullets(body):
    return [l[2:].strip() for l in body.split("\n") if l.startswith("- ")]


def build():
    md = SRC.read_text()
    top = sections(md, 2)
    goal = top.get("Goal", "")
    nxt = top.get("Do this next", "")
    nxt_title = next((l for l in nxt.split("\n") if l.startswith("**")), "")
    nxt_points = bullets(nxt)

    where = table(top.get("Where we are", ""))
    columns = {h: [] for h in where[0]} if where else {}
    for row in where[1:]:
        for head, cell in zip(where[0], row):
            if cell and cell not in ("—", ""):
                columns[head].append(cell)

    journey = top.get("The journey", "")
    mermaid = re.search(r"```mermaid\n(.*?)```", journey, re.S)
    mermaid = mermaid.group(1) if mermaid else ""
    steps = re.search(r"<details>.*?</summary>(.*?)</details>", journey, re.S)
    steps = bullets(steps.group(1).strip()) if steps else []

    groups = sections(top.get("Tasks", ""), 3)
    tasks_html, buckets, statuses = [], set(), set()
    for name, body in groups.items():
        rows = table(body)
        if not rows:
            continue
        cards = []
        for cells in rows[1:]:
            task, status, bucket, payoff, size, who, ident = (cells + [""] * 7)[:7]
            cls = status_class(status)
            buckets.add(bucket)
            statuses.add(cls)
            detail = re.sub(r"^\**[^:·]*\**\s*[·:]?\s*", "", status, count=1) if ":" in status or "·" in status else ""
            cards.append(
                f'<div class="task" data-bucket="{html.escape(bucket)}" data-status="{cls}">'
                f'<div class="row"><span class="tag {cls}">{html.escape(status_word(status))}</span>'
                f'<span class="title">{inline(task)}</span></div>'
                + (f'<div class="detail">{inline(detail)}</div>' if detail else "")
                + f'<div class="chips"><span>{html.escape(bucket)}</span><span>{inline(payoff)}</span>'
                f'<span>{html.escape(size)}</span><span>{inline(who)}</span>'
                f'<span class="id">{inline(ident)}</span></div></div>')
        open_attr = " open" if any("now" in c or "next" in c for c in cards) else ""
        tasks_html.append(f'<details class="group"{open_attr}><summary>{html.escape(name)}'
                          f' <small>{len(cards)} tasks</small></summary>{"".join(cards)}</details>')

    history = bullets(top.get("History", ""))
    parked = top.get("Parked ideas", "")
    scoring = table(top.get("Scoring details (for agents)", ""))

    def col(head):
        items = "".join(f'<div class="ms">{inline(c)}</div>' for c in columns.get(head, []))
        return f'<div class="col"><h3>{head}</h3>{items or "<p class=muted>Nothing yet</p>"}</div>'

    score_rows = "".join("<tr>" + "".join(f"<td>{inline(c)}</td>" for c in r) + "</tr>" for r in scoring[1:])
    score_head = "".join(f"<th>{html.escape(c)}</th>" for c in (scoring[0] if scoring else []))
    status_names = {"next": "Do this next", "now": "In progress", "ready": "Ready",
                    "wait": "Waiting", "done": "Done", "dropped": "Dropped"}
    bucket_buttons = "".join(f'<button data-f="bucket" data-v="{html.escape(b)}">{html.escape(b)}</button>'
                             for b in sorted(buckets))
    status_buttons = "".join(f'<button data-f="status" data-v="{s}">{status_names[s]}</button>'
                             for s in status_names if s in statuses)

    page = f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Toasty roadmap</title>
<style>
:root {{ --bg:#fafaf9; --card:#fff; --ink:#1c1917; --muted:#78716c; --line:#e7e5e4;
  --next:#b45309; --next-bg:#fef3c7; --now:#1d4ed8; --now-bg:#dbeafe; --ready:#047857; --ready-bg:#d1fae5;
  --wait:#6b7280; --wait-bg:#f3f4f6; --done:#065f46; --done-bg:#a7f3d0; --drop:#b91c1c; --drop-bg:#fee2e2; }}
@media (prefers-color-scheme: dark) {{ :root {{ --bg:#0c0a09; --card:#1c1917; --ink:#f5f5f4; --muted:#a8a29e;
  --line:#292524; --next:#fbbf24; --next-bg:#451a03; --now:#93c5fd; --now-bg:#1e3a8a; --ready:#6ee7b7;
  --ready-bg:#064e3b; --wait:#9ca3af; --wait-bg:#1f2937; --done:#6ee7b7; --done-bg:#065f46; --drop:#fca5a5; --drop-bg:#450a0a; }} }}
* {{ box-sizing:border-box }} body {{ margin:0; background:var(--bg); color:var(--ink);
  font:15px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif }}
main {{ max-width:1000px; margin:0 auto; padding:28px 20px 64px }}
h1 {{ font-size:26px; margin:0 }} h2 {{ font-size:17px; margin:34px 0 12px }}
.muted, small {{ color:var(--muted) }} a {{ color:var(--now) }}
code {{ font:12.5px ui-monospace,Menlo,monospace; background:var(--wait-bg); padding:1px 5px; border-radius:5px }}
.goal {{ color:var(--muted); margin:4px 0 20px }}
.next {{ background:var(--next-bg); border-radius:16px; padding:18px 22px }}
.next .label {{ font-size:12px; font-weight:700; letter-spacing:.06em; text-transform:uppercase; color:var(--next) }}
.next .what {{ font-size:19px; font-weight:650; margin:4px 0 8px }} .next ul {{ margin:0; padding-left:18px }}
.cols {{ display:grid; grid-template-columns:repeat(4,1fr); gap:10px }}
@media (max-width:760px) {{ .cols {{ grid-template-columns:1fr 1fr }} }}
.col {{ background:var(--card); border:1px solid var(--line); border-radius:12px; padding:10px 12px }}
.col h3 {{ margin:0 0 8px; font-size:13px; color:var(--muted) }}
.ms {{ background:var(--bg); border-radius:8px; padding:7px 9px; margin-bottom:6px; font-size:14px }}
.chart {{ background:var(--card); border:1px solid var(--line); border-radius:12px; padding:14px; overflow-x:auto }}
.steps {{ margin:10px 0 0; padding-left:18px; font-size:14px; color:var(--muted) }}
.filters {{ display:flex; flex-wrap:wrap; gap:6px; margin-bottom:12px }}
.filters button {{ border:1px solid var(--line); background:var(--card); color:var(--ink); border-radius:999px;
  padding:3px 12px; font-size:13px; cursor:pointer }} .filters button.on {{ background:var(--ink); color:var(--bg) }}
details.group {{ background:var(--card); border:1px solid var(--line); border-radius:12px; padding:0 14px; margin-bottom:10px }}
details.group > summary {{ cursor:pointer; padding:12px 0; font-weight:650 }}
.task {{ border-top:1px solid var(--line); padding:10px 0 }}
.row {{ display:flex; gap:10px; align-items:baseline }} .title {{ font-weight:550 }}
.detail {{ font-size:13.5px; color:var(--muted); margin:3px 0 0 2px }}
.chips {{ display:flex; flex-wrap:wrap; gap:5px; margin-top:6px }}
.chips span {{ font-size:12px; border:1px solid var(--line); border-radius:999px; padding:0 8px; color:var(--muted) }}
.tag {{ flex:none; font-size:12px; font-weight:700; border-radius:999px; padding:1px 9px; white-space:nowrap }}
.tag.next {{ background:var(--next-bg); color:var(--next) }} .tag.now {{ background:var(--now-bg); color:var(--now) }}
.tag.ready {{ background:var(--ready-bg); color:var(--ready) }} .tag.wait {{ background:var(--wait-bg); color:var(--wait) }}
.tag.done {{ background:var(--done-bg); color:var(--done) }} .tag.dropped {{ background:var(--drop-bg); color:var(--drop) }}
ul.history {{ padding-left:18px }} ul.history li {{ margin-bottom:6px }}
table {{ border-collapse:collapse; font-size:12.5px; width:100% }} td,th {{ border-bottom:1px solid var(--line); padding:4px 6px; text-align:left }}
.foot {{ margin-top:30px; font-size:13px; color:var(--muted) }}
</style></head><body><main>
<h1>Toasty roadmap</h1>
<p class="goal">{inline(goal)}</p>
<div class="next"><div class="label">Do this next</div>
<div class="what">{inline(nxt_title)}</div><ul>{"".join(f"<li>{inline(p)}</li>" for p in nxt_points)}</ul></div>
<h2>Where we are</h2>
<div class="cols">{col("Done")}{col("Now")}{col("Up next")}{col("Later")}</div>
<h2>The journey</h2>
<div class="chart"><pre class="mermaid">{html.escape(mermaid)}</pre>
<details><summary class="muted">Steps inside each milestone</summary>
<ul class="steps">{"".join(f"<li>{inline(s)}</li>" for s in steps)}</ul></details></div>
<h2>Tasks</h2>
<div class="filters"><button data-f="all" class="on">All</button>{status_buttons}{bucket_buttons}</div>
{"".join(tasks_html)}
<h2>History</h2>
<ul class="history">{"".join(f"<li>{inline(h)}</li>" for h in history)}</ul>
<h2>Parked ideas</h2><p>{inline(parked)}</p>
<details class="group"><summary>Scoring details (for agents)</summary>
<table><tr>{score_head}</tr>{score_rows}</table></details>
<p class="foot">Built from ROADMAP.md by tools/roadmap_page.py. The filters only change what this page shows; nothing here saves.</p>
</main>
<script type="module">
import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";
mermaid.initialize({{ startOnLoad: true, theme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "default" }});
</script>
<script>
const buttons = document.querySelectorAll(".filters button");
buttons.forEach(b => b.addEventListener("click", () => {{
  buttons.forEach(x => x.classList.toggle("on", x === b));
  document.querySelectorAll(".task").forEach(t => {{
    const show = b.dataset.f === "all" || t.dataset[b.dataset.f] === b.dataset.v;
    t.style.display = show ? "" : "none";
  }});
  document.querySelectorAll("details.group").forEach(g => {{
    if (b.dataset.f !== "all" && g.querySelector(".task:not([style*='none'])")) g.open = true;
  }});
}}));
</script>
</body></html>
"""
    OUT.write_text(page)
    return OUT


if __name__ == "__main__":
    print(build())
    sys.exit(0)
