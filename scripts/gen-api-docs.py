#!/usr/bin/env python3
"""Generate docs/API.md: the public API of the `fe_zk` ingot, from the source.

Every name re-exported by a `pub use` in src/lib.fe is looked up in src/ (not src/tests), and its
declaration line(s) and the first line of its `///` doc comment are written to docs/API.md, grouped
by module. A re-exported verifier module (`pub use verifier::{groth16, ...}`) gets its own section
listing the public items of that module's own file(s), including the `pub fn` methods of its
inherent `impl` blocks (`Vk::from_consts`). Each name is resolved inside its own module, so two
modules that both export `verify` never shadow each other. Nothing is hand-written, so the file
cannot drift from the code:

    python3 -I scripts/gen-api-docs.py            # rewrite docs/API.md
    python3 -I scripts/gen-api-docs.py --check    # exit 1 if docs/API.md is out of date (CI)

A bare `pub use anon` (a module directory) gets its own section listing the public items of every file
under src/anon/. The `semaphore/` ingot (a separate ingot, `fe_zk_semaphore`) is listed in its own
section from semaphore/src/lib.fe: its public functions and the methods of its public types.

Names the script cannot resolve are an error (a new re-export must have a findable declaration).
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


def extract(lines, ln, kind):
    """Signature (one line) and first doc sentence of the declaration that starting at lines[ln]."""
    sig = []
    i = ln
    while i < len(lines):
        sig.append(lines[i].strip())
        if re.search(r"\{\s*$", lines[i]) or re.search(r"=\s*\S", lines[i]) and kind in ("const", "type"):
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
    if kind == "const":
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
    mm = re.match(r"(.+?[.!?])(\s|$)", " ".join(doc).strip()) if doc else None
    if mm:
        first = mm.group(1)
    return sigtxt, first


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
        sigtxt, first = extract(lines, ln, m.group(1))
        return os.path.relpath(f, ROOT), m.group(1), sigtxt, first
    return None


TOP = re.compile(r"^pub\s+(?:const\s+)?(?:unsafe\s+)?(fn|struct|trait|enum|type)\s+(\w+)")
IMPL = re.compile(r"^impl(?:<[^>]*>)?\s+(\w+)")
METHOD = re.compile(r"^    pub\s+(?:const\s+)?(?:unsafe\s+)?fn\s+(\w+)")


def module_items(files):
    """Public items of one module (its own files only): top-level declarations, and the `pub fn`
    methods of inherent impls as `Type::name`. Returns [(name, file, kind, sig, doc)]."""
    out = []
    for f in files:
        lines = read(f).split("\n")
        owner = None
        for ln, line in enumerate(lines):
            m = IMPL.match(line)
            if m and " for " not in line:
                owner = m.group(1)
            elif line.startswith("}"):
                owner = None
            m = TOP.match(line)
            if m:
                sig, doc = extract(lines, ln, m.group(1))
                out.append((m.group(2), os.path.relpath(f, ROOT), m.group(1), sig, doc))
                continue
            m = METHOD.match(line)
            if m and owner:
                sig, doc = extract(lines, ln, "fn")
                out.append((f"{owner}::{m.group(1)}", os.path.relpath(f, ROOT), "fn", sig, doc))
    return out


def module_files(mod):
    """Files of the verifier module `mod`: its own file, plus `halo2/common.fe`. Generated
    per-circuit modules and the FRI configuration submodules are not part of the listing."""
    base = os.path.join(SRC, "verifier", mod)
    files = [base + ".fe"]
    if mod == "halo2":
        files.append(os.path.join(base, "common.fe"))
    return files


def anon_files(mod):
    d = os.path.join(SRC, mod)
    return sorted(os.path.join(d, f) for f in os.listdir(d) if f.endswith(".fe"))


def main():
    lib = read(os.path.join(SRC, "lib.fe"))
    for m in re.finditer(r"^pub use (\w+)\s*$", lib, re.M):
        if os.path.isdir(os.path.join(SRC, m.group(1))):
            mods_dir = m.group(1)
            break
    else:
        mods_dir = None
    groups = []  # (module, [(name, decl)])
    mods = []  # (path, items) for re-exported modules
    seen = set()
    for path, items in uses(lib):
        for orig, name in items:
            if name in seen:
                continue
            seen.add(name)
            if os.path.exists(os.path.join(SRC, *path.split("::"), orig + ".fe")):
                mods.append((path + "::" + orig, module_items(module_files(orig))))
                continue
            d = find_decl(orig, path)
            if d is None:
                sys.exit(f"gen-api-docs: cannot find the declaration of {name} (as {orig}) re-exported from {path}")
            for g in groups:
                if g[0] == path:
                    g[1].append((name, orig, d))
                    break
            else:
                groups.append((path, [(name, orig, d)]))
    out = []
    sem = module_items([os.path.join(ROOT, "semaphore", "src", "lib.fe")])
    out.append("# fe_zk public API\n")
    out.append("<!-- GENERATED by scripts/gen-api-docs.py from src/lib.fe and the declarations it re-exports. Do not edit;")
    out.append("     CI fails if this file is out of date. -->\n")
    out.append("Every name in the field, ec and hash sections is re-exported at the root of the `fe_zk` ingot")
    out.append("(`use fe_zk::<Name>`). The verifiers are modules, one per proof system (`use fe_zk::groth16`, then")
    out.append("`groth16::verify(..)`). `anon` is a module of building blocks for anonymous actions; the Semaphore ingot is a separate")
    out.append("package, listed last. Signatures are copied from the source; the description is the first sentence")
    out.append("of the declaration's doc comment. Safety rules and their enforcement: [SAFETY.md](SAFETY.md).\n")
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
    for mod, items in mods:
        short = mod.split("::")[-1]
        out.append(f"## `{mod}`\n")
        out.append(f"After `use fe_zk::{short}`, call `{short}::<name>`.\n")
        out.append("| Name | Kind | Signature | Description |")
        out.append("|---|---|---|---|")
        for name, f, kind, sig, doc in items:
            sig = sig.replace("|", "\\|")
            doc = doc.replace("|", "\\|")
            out.append(f"| `{name}` | {kind} | `{sig}` | {doc or '-'} [`{f}`](../{f}) |")
        out.append("")
    if mods_dir:
        out.append(f"## `{mods_dir}`\n")
        out.append(f"After `use fe_zk::{mods_dir}`, or `use fe_zk::{mods_dir}::{{LeanImt, Nullifiers, hash_to_field}}`.\n")
        out.append("| Name | Kind | Signature | Description |")
        out.append("|---|---|---|---|")
        for name, f, kind, sig, doc in module_items(anon_files(mods_dir)):
            sig = sig.replace("|", "\\|")
            doc = doc.replace("|", "\\|")
            out.append(f"| `{name}` | {kind} | `{sig}` | {doc or '-'} [`{f}`](../{f}) |")
        out.append("")
    out.append("## `fe_zk_semaphore` (the `semaphore/` ingot)\n")
    out.append("A separate ingot: add `fe_zk_semaphore = { path = \"../semaphore\" }` to `fe.toml` and `use fe_zk_semaphore::verify_proof_low`.")
    out.append("Semaphore v4 proof verification with the published ceremony keys, tree depths 1 to 10 (see")
    out.append("[`semaphore/README.md`](../semaphore/README.md)).\n")
    out.append("| Name | Kind | Signature | Description |")
    out.append("|---|---|---|---|")
    for name, f, kind, sig, doc in sem:
        sig = sig.replace("|", "\\|")
        doc = doc.replace("|", "\\|")
        out.append(f"| `{name}` | {kind} | `{sig}` | {doc or '-'} [`{f}`](../{f}) |")
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
