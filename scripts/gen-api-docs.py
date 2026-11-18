#!/usr/bin/env python3
"""Generate docs/API.md: the public API of the `fe_zk` ingot, from the source.

Every name re-exported by a `pub use` in src/lib.fe is looked up in src/ (not src/tests), and its
declaration line(s) and the first line of its `///` doc comment are written to docs/API.md, grouped
by module. Nothing is hand-written, so the file cannot drift from the code:

    python3 -I scripts/gen-api-docs.py            # rewrite docs/API.md
    python3 -I scripts/gen-api-docs.py --check    # exit 1 if docs/API.md is out of date (CI)

Names the script cannot resolve are an error (a new re-export must have a findable declaration).
Library modules that are not re-exported from lib.fe are reachable by path (`fe_zk::verifier::X`)
and are listed in the "Also reachable by path" section when a module re-exports them (src/<mod>.fe).
"""
import os
import re
import sys

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
SRC = os.path.join(ROOT, "src")
OUT = os.path.join(ROOT, "docs", "API.md")


def read(p):
    with open(p, encoding="utf-8") as f:
        return f.read()


def uses(text):
    """Yield (path, [(name, alias)]) for every `pub use` in text."""
    for m in re.finditer(r"^pub use ([\w:]+?)(?:::\{([^}]*)\}|::(\w+))\s*$", text, re.M):
        path = m.group(1)
        names = m.group(2) if m.group(2) is not None else m.group(3)
        items = []
        for n in names.split(","):
            n = n.strip()
            if not n:
                continue
            if " as " in n:
                a, b = n.split(" as ")
                items.append((a.strip(), b.strip()))
            else:
                items.append((n, n))
        yield path, items


def src_files():
    out = []
    for d, _, fs in os.walk(SRC):
        if os.path.relpath(d, SRC).split(os.sep)[0] == "tests":
            continue
        for f in fs:
            if f.endswith(".fe") and f not in ("tests.fe",):
                out.append(os.path.join(d, f))
    return sorted(out)


FILES = src_files()
DEF = r"^(?:pub\s+)(?:const\s+)?(?:unsafe\s+)?(fn|struct|trait|enum|type|const|msg|contract)\s+{}\b"


def find_decl(name, hint_path):
    """Return (file, kind, signature, doc) of the top-level `pub` declaration of name."""
    rx = re.compile(DEF.format(re.escape(name)), re.M)
    # prefer files under the module the name is re-exported from
    hint = os.path.join(SRC, *hint_path.split("::")[:1])
    cands = sorted(FILES, key=lambda f: (not f.startswith(hint), f))
    for f in cands:
        text = read(f)
        m = rx.search(text)
        if not m:
            continue
        lines = text.split("\n")
        ln = text[: m.start()].count("\n")
        sig = []
        i = ln
        while i < len(lines):
            sig.append(lines[i].strip())
            joined = " ".join(sig)
            if re.search(r"\{\s*$", lines[i]) or re.search(r"=\s*\S", lines[i]) and m.group(1) in ("const", "type"):
                break
            if m.group(1) in ("fn",) and lines[i].rstrip().endswith("{"):
                break
            i += 1
            if i - ln > 40:
                break
        sigtxt = " ".join(sig)
        sigtxt = re.sub(r"\s*\{.*$", "", sigtxt)
        sigtxt = re.sub(r"\(\s+", "(", sigtxt)
        sigtxt = re.sub(r",\s*\)", ")", sigtxt)
        sigtxt = re.sub(r"\s+", " ", sigtxt).strip()
        sigtxt = re.sub(r",\s*(?=\)|>|\])", "", sigtxt)
        if m.group(1) == "const":
            sigtxt = re.sub(r"\s*=.*$", "", sigtxt)
        # doc comment above (skip attributes)
        j = ln - 1
        while j >= 0 and lines[j].strip().startswith("#["):
            j -= 1
        doc = []
        while j >= 0 and lines[j].strip().startswith("///"):
            doc.append(lines[j].strip()[3:].strip())
            j -= 1
        doc.reverse()
        first = ""
        for d in doc:
            if d:
                first = d
                break
        # first sentence only
        mm = re.match(r"(.+?[.!?])(\s|$)", " ".join(doc).strip()) if doc else None
        if mm:
            first = mm.group(1)
        return os.path.relpath(f, ROOT), m.group(1), sigtxt, first
    return None


def main():
    lib = read(os.path.join(SRC, "lib.fe"))
    # alias map from the module files (e.g. `verify as groth16_verify` in src/verifier.fe)
    alias = {}
    for f in FILES:
        for _, items in uses(read(f)):
            for a, b in items:
                if a != b:
                    alias[b] = a
    groups = []  # (module, [(name, decl)])
    seen = set()
    for path, items in uses(lib):
        for orig, name in items:
            if name in seen:
                continue
            seen.add(name)
            real = alias.get(name, orig)
            d = find_decl(real, path)
            if d is None:
                sys.exit(f"gen-api-docs: cannot find the declaration of {name} (as {real}) re-exported from {path}")
            mod = path
            for g in groups:
                if g[0] == mod:
                    g[1].append((name, real, d))
                    break
            else:
                groups.append((mod, [(name, real, d)]))
    out = []
    out.append("# fe_zk public API\n")
    out.append("<!-- GENERATED by scripts/gen-api-docs.py from src/lib.fe and the declarations it re-exports. Do not edit;")
    out.append("     CI fails if this file is out of date. -->\n")
    out.append("Every name below is re-exported at the root of the `fe_zk` ingot (`use fe_zk::<Name>`). Signatures are")
    out.append("copied from the source; the description is the first sentence of the declaration's doc comment.")
    out.append("Modules whose items are also reachable by path (`fe_zk::verifier::...`, `fe_zk::ec::...`) are listed in")
    out.append("`src/<module>.fe`. Safety rules and their enforcement: [SAFETY.md](SAFETY.md).\n")
    order = sorted(groups, key=lambda g: g[0].split("::")[0] != "field")
    for mod, items in order:
        out.append(f"## `{mod}`\n")
        out.append("| Name | Kind | Signature | Description |")
        out.append("|---|---|---|---|")
        for name, real, (f, kind, sig, doc) in sorted(items, key=lambda x: x[0].lower()):
            sig = sig.replace("|", "\\|")
            doc = doc.replace("|", "\\|")
            label = name if name == real else f"{name} (`{real}`)"
            out.append(f"| `{label}` | {kind} | `{sig}` | {doc or '-'} [`{f}`](../{f}) |")
        out.append("")
    text = "\n".join(out)
    if "--check" in sys.argv:
        cur = read(OUT) if os.path.exists(OUT) else ""
        if cur != text:
            sys.stderr.write("docs/API.md is out of date: run python3 -I scripts/gen-api-docs.py\n")
            sys.exit(1)
        print("docs/API.md is up to date")
        return
    with open(OUT, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {os.path.relpath(OUT, ROOT)}: {len(seen)} names")


main()
