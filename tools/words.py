#!/usr/bin/env python3
"""Keeps Lounge's translations in step with its source text. Run from anywhere:

    tools/words.py check             every catalog is complete and well formed (CI runs this)
    tools/words.py sync              rewrite the catalogs to match the source, keeping translations
    tools/words.py sync --add sv     the same, and start catalogs for a new language
    tools/words.py check plugins/weather    one plugin, by its directory
    tools/words.py sync core         one of Lounge's own catalogs: core, sdk or phone

docs/TRANSLATING.md says how text is marked and how a language is added."""
import difflib
import glob
import os
import re
import sys
import xml.etree.ElementTree as ET

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NOT_TRANSLATED = {"sdk", "lgp-tool", "target"}
PLACE = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}")

# The plural rule gettext tools expect in a catalog's header, by language.
PLURAL_HEADERS = {
    "fr": "nplurals=2; plural=(n > 1);",
    "fil": "nplurals=2; plural=(n > 1);",
    "pt": "nplurals=2; plural=(n > 1);",
    "pt-BR": "nplurals=2; plural=(n > 1);",
    "pt-PT": "nplurals=2; plural=(n != 1);",
    "ja": "nplurals=1; plural=0;",
    "ko": "nplurals=1; plural=0;",
    "zh": "nplurals=1; plural=0;",
    "ru": "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
    "uk": "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
    "pl": "nplurals=3; plural=(n==1 ? 0 : n%10>=2 && n%10<=4 && (n%100<10 || n%100>=20) ? 1 : 2);",
    "cs": "nplurals=3; plural=(n==1 ? 0 : n>=2 && n<=4 ? 1 : 2);",
    "sk": "nplurals=3; plural=(n==1 ? 0 : n>=2 && n<=4 ? 1 : 2);",
    "ro": "nplurals=3; plural=(n==1 ? 0 : (n==0 || (n%100>0 && n%100<20)) ? 1 : 2);",
    "lt": "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n%10>=2 && (n%100<10 || n%100>=20) ? 1 : 2);",
    "lv": "nplurals=3; plural=(n%10==1 && n%100!=11 ? 0 : n!=0 ? 1 : 2);",
    "sl": "nplurals=4; plural=(n%100==1 ? 0 : n%100==2 ? 1 : n%100==3 || n%100==4 ? 2 : 3);",
    "ar": "nplurals=6; plural=(n==0 ? 0 : n==1 ? 1 : n==2 ? 2 : n%100>=3 && n%100<=10 ? 3 : n%100>=11 ? 4 : 5);",
}
DEFAULT_PLURAL_HEADER = "nplurals=2; plural=(n != 1);"

# The quantities Android wants in a <plurals> for each language; any other language needs one and other.
ANDROID_QUANTITIES = {"es": ["one", "many", "other"], "fr": ["one", "many", "other"], "it": ["one", "many", "other"], "pt": ["one", "many", "other"]}


def plural_header(lang):
    return PLURAL_HEADERS.get(lang) or PLURAL_HEADERS.get(lang.split("-")[0]) or DEFAULT_PLURAL_HEADER


def plural_count(lang):
    return int(re.search(r"nplurals=(\d+)", plural_header(lang)).group(1))


# ----- reading source -----

class Message:
    def __init__(self, context, text, plural):
        self.context, self.text, self.plural = context, text, plural
        self.files, self.notes = [], []

    @property
    def key(self):
        return (self.context, self.text)


def blank_comments(src, rust):
    """`src` with comments turned to spaces, so text inside them is never taken for a call."""
    out, i, n = list(src), 0, len(src)

    def blank(a, b):
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif src.startswith("/*", i):
            j = src.find("*/", i + 2)
            j = n if j < 0 else j + 2
            blank(i, j)
            i = j
        elif not rust and c == "/" and regex_starts(src, i):
            i = end_of_regex(src, i)
        elif not rust and src.startswith("<!--", i):
            j = src.find("-->", i + 4)
            j = n if j < 0 else j + 3
            blank(i, j)
            i = j
        elif rust and c == "r" and re.match(r'r#*"', src[i:i + 12]) and (i == 0 or not (src[i - 1].isalnum() or src[i - 1] == "_")):
            hashes = len(re.match(r"r(#*)", src[i:]).group(1))
            j = src.find('"' + "#" * hashes, i + 2 + hashes)
            i = n if j < 0 else j + 1 + hashes
        elif c == '"' or (not rust and c in "'`"):
            i = end_of_string(src, i)
        elif rust and c == "'":
            # A char literal, or a lifetime, which has no closing quote.
            m = re.match(r"'(\\.[^']*|[^\\'])'", src[i:i + 12])
            i += len(m.group(0)) if m else 1
        else:
            i += 1
    return "".join(out)


def regex_starts(src, i):
    """Whether the slash at `i` opens a regular expression, going by what stands before it."""
    j = i - 1
    while j >= 0 and src[j] in " \t\n":
        j -= 1
    return j < 0 or src[j] in "(,=:[!&|?{};+-*%<>~^" or src[:j + 1].endswith(("return", "typeof", "case"))


def end_of_regex(src, i):
    j, in_class = i + 1, False
    while j < len(src) and src[j] != "\n":
        c = src[j]
        if c == "\\":
            j += 2
            continue
        if c == "[":
            in_class = True
        elif c == "]":
            in_class = False
        elif c == "/" and not in_class:
            return j + 1
        j += 1
    return i + 1


def end_of_string(src, i):
    quote, j = src[i], i + 1
    while j < len(src) and src[j] != quote:
        j += 2 if src[j] == "\\" else 1
    return j + 1


RUST_ESCAPES = {"n": "\n", "t": "\t", "r": "\r", "0": "\0", "\\": "\\", '"': '"', "'": "'"}


def read_literal(src, i, rust, where):
    """The string literal at `i` and the index after it, or (None, i) if there is none."""
    while i < len(src) and src[i].isspace():
        i += 1
    if rust and re.match(r'r#*"', src[i:i + 12]):
        hashes = len(re.match(r"r(#*)", src[i:]).group(1))
        j = src.find('"' + "#" * hashes, i + 2 + hashes)
        return src[i + 2 + hashes:j], j + 1 + hashes
    if i >= len(src) or src[i] not in ('"' if rust else "\"'`"):
        return None, i
    end = end_of_string(src, i)
    body, out, k = src[i + 1:end - 1], [], 0
    if src[i] == "`" and "${" in body:
        sys.exit(f"{where}: a template string with ${{}} cannot be translated; use a {{name}} place")
    while k < len(body):
        c = body[k]
        if c != "\\":
            out.append(c)
            k += 1
            continue
        e = body[k + 1]
        if e == "u" or e == "x":
            m = re.match(r"u\{([0-9a-fA-F]+)\}|u([0-9a-fA-F]{4})|x([0-9a-fA-F]{2})", body[k + 1:])
            if not m:
                sys.exit(f"{where}: an escape that names no character")
            out.append(chr(int(m.group(1) or m.group(2) or m.group(3), 16)))
            k += 1 + len(m.group(0))
        elif e == "\n":
            # A backslash at the end of a line joins it to the next, without the indent.
            k += 2
            while k < len(body) and body[k].isspace():
                k += 1
        else:
            out.append(RUST_ESCAPES.get(e, e))
            k += 2
    return "".join(out), end


def skip_argument(src, i):
    """The index after the comma that ends the expression starting at `i`."""
    depth = 0
    while i < len(src):
        c = src[i]
        if c in "\"'`":
            i = end_of_string(src, i) if c != "'" or not re.match(r"'[a-z_]+\b(?!')", src[i:i + 20]) else i + 1
            continue
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            return i + 1
        i += 1
    return i


def without_test_modules(src):
    """Rust source with each `#[cfg(test)] mod` turned to spaces, comments already blanked."""
    out, at = [], 0
    for m in re.finditer(r"#\[cfg\(test\)\]\s*mod\s+\w+\s*\{", src):
        if m.start() < at:
            continue
        i, depth = m.end(), 1
        while i < len(src) and depth:
            c = src[i]
            raw = re.match(r'r(#*)"', src[i:i + 12]) if c == "r" else None
            if raw:
                close = src.find('"' + raw.group(1), i + len(raw.group(0)))
                i = len(src) if close < 0 else close + 1 + len(raw.group(1))
                continue
            if c == '"':
                i = end_of_string(src, i)
                continue
            if c == "'":
                char = re.match(r"'(\\.[^']*|[^\\'])'", src[i:i + 12])
                i += len(char.group(0)) if char else 1
                continue
            depth += (c == "{") - (c == "}")
            i += 1
        out.append(src[at:m.start()])
        out.append(re.sub(r"[^\n]", " ", src[m.start():i]))
        at = i
    out.append(src[at:])
    return "".join(out)


def messages_in(path, shown):
    rust = path.endswith(".rs")
    raw = open(path, encoding="utf-8").read()
    src = blank_comments(raw, rust)
    # Tests are written in English and never shown, so text in a test module is left out.
    if rust:
        src = without_test_modules(src)
    lines = raw.split("\n")
    call = re.compile(r"\b(t|tc|tn|mark)!\s*\(" if rust else r"(?<![\w$.])(t|tc|tn|mark)\(")
    found = []
    for m in call.finditer(src):
        kind, i = m.group(1), m.end()
        line = src.count("\n", 0, m.start())
        where = f"{shown}:{line + 1}"
        context = plural = None
        if kind == "tc":
            context, i = read_literal(src, i, rust, where)
            if context is None:
                continue
            i = skip_argument(src, i)
        if kind == "tn":
            i = skip_argument(src, i)
        text, i = read_literal(src, i, rust, where)
        if text is None:
            if rust:
                sys.exit(f"{where}: {kind}! needs its text written out, so translators can see it")
            continue
        if kind == "tn":
            plural, i = read_literal(src, skip_argument(src, i), rust, where)
            if plural is None:
                sys.exit(f"{where}: tn needs the wording for one and for many")
        message = Message(context, text, plural)
        message.files.append(shown)
        k = line - 1
        while k >= 0 and re.match(r"\s*(//|<!--)\s*Translators:", lines[k]):
            message.notes.insert(0, re.sub(r"^\s*(//|<!--)\s*Translators:\s*|\s*-->\s*$", "", lines[k]))
            k -= 1
        found.append(message)
    return found


def source_messages(files, base):
    merged = {}
    for path in sorted(files):
        for message in messages_in(path, os.path.relpath(path, base)):
            have = merged.setdefault(message.key, message)
            if have is not message:
                if have.plural != message.plural:
                    sys.exit(f"{message.files[0]}: \"{message.text}\" is counted in one place and not in another, or with other words; give one a context")
                have.files += [f for f in message.files if f not in have.files]
                have.notes += [note for note in message.notes if note not in have.notes]
    return list(merged.values())


# ----- reading and writing catalogs -----

def unquote(s):
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t"}.get(m.group(1), m.group(1)), s[1:-1])


def quote(s):
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n").replace("\t", "\\t") + '"'


def read_po(path):
    """A catalog as {(context, text): entry}; each entry holds its forms, flags and translator's comments."""
    entries, entry, field = {}, None, None

    def close():
        if entry and entry["id"]:
            entries[(entry["context"], entry["id"])] = entry

    if not os.path.exists(path):
        return entries
    for line in open(path, encoding="utf-8").read().split("\n") + [""]:
        line = line.strip()
        if not line:
            close()
            entry, field = None, None
            continue
        entry = entry or {"context": None, "id": "", "plural": None, "forms": [], "fuzzy": False, "comments": []}
        if line.startswith("#"):
            if line.startswith("#,") and "fuzzy" in line:
                entry["fuzzy"] = True
            elif line == "#" or line.startswith("# "):
                entry["comments"].append(line)
            continue
        if not line.startswith('"'):
            keyword, _, line = line.partition(" ")
            field = keyword
        piece = unquote(line.strip())
        if field == "msgctxt":
            entry["context"] = (entry["context"] or "") + piece
        elif field == "msgid":
            entry["id"] += piece
        elif field == "msgid_plural":
            entry["plural"] = (entry["plural"] or "") + piece
        elif field and field.startswith("msgstr"):
            index = int(field[7:-1]) if "[" in field else 0
            while len(entry["forms"]) <= index:
                entry["forms"].append("")
            entry["forms"][index] += piece
    return entries


def nearest(message, gone):
    """The dropped entry whose English is closest to `message`, if any is close: its wording as it was before an edit."""
    best, score = None, 0.6
    for key, entry in gone.items():
        if key[0] != message.context or (entry["plural"] is None) != (message.plural is None) or not any(entry["forms"]):
            continue
        ratio = difflib.SequenceMatcher(None, key[1], message.text).ratio()
        if ratio > score:
            best, score = key, ratio
    return best


def write_po(path, title, lang, messages, old):
    out = [f"# {title}", "#", 'msgid ""', 'msgstr ""', quote(f"Language: {lang}\n"), quote("Content-Type: text/plain; charset=UTF-8\n"), quote(f"Plural-Forms: {plural_header(lang)}\n"), ""]
    keys = {message.key for message in messages}
    gone = {key: entry for key, entry in old.items() if key not in keys}
    for message in messages:
        had = old.get(message.key)
        if had is None:
            # The English was edited: its old translation comes along, marked as needing a look.
            was = nearest(message, gone)
            had = dict(gone.pop(was), fuzzy=True) if was else {}
        out += had.get("comments", [])
        out += [f"#. {note}" for note in message.notes]
        out.append("#: " + " ".join(message.files))
        forms = list(had.get("forms", []))
        # Translated from other English than this, so it needs looking at again.
        if had.get("fuzzy") or (forms and any(forms) and had.get("plural") != message.plural):
            out.append("#, fuzzy")
        if message.context is not None:
            out.append(f"msgctxt {quote(message.context)}")
        out.append(f"msgid {quote(message.text)}")
        if message.plural is None:
            out.append(f"msgstr {quote(forms[0] if forms else '')}")
        else:
            out.append(f"msgid_plural {quote(message.plural)}")
            count = plural_count(lang)
            forms = (forms + [""] * count)[:count]
            out += [f"msgstr[{i}] {quote(form)}" for i, form in enumerate(forms)]
        out.append("")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    open(path, "w", encoding="utf-8").write("\n".join(out))


# ----- the catalogs there are -----

class Catalog:
    def __init__(self, name, base, sources, locales):
        self.name, self.base, self.locales = name, base, locales
        self.files = [f for pattern in sources for f in glob.glob(os.path.join(ROOT, pattern), recursive=True)]

    def languages(self):
        return sorted(os.path.basename(p)[:-3] for p in glob.glob(os.path.join(ROOT, self.locales, "*.po")))


def plugin_catalog(directory):
    rel = os.path.relpath(os.path.abspath(directory), ROOT)
    return Catalog(os.path.basename(rel), rel, [f"{rel}/src/**/*.rs"], f"{rel}/locales")


def catalogs():
    found = [
        Catalog("core", "core", ["core/lounge-*/src/**/*.rs"], "core/locales"),
        Catalog("plugin SDK", ".", ["plugins/sdk/src/**/*.rs", "core/lounge-words/src/**/*.rs"], "plugins/sdk/locales"),
        Catalog("phone", "companion", ["companion/src/**/*.ts", "companion/src/**/*.svelte"], "companion/locales"),
    ]
    for cargo in sorted(glob.glob(os.path.join(ROOT, "plugins/*/Cargo.toml"))):
        if os.path.basename(os.path.dirname(cargo)) not in NOT_TRANSLATED:
            found.append(plugin_catalog(os.path.dirname(cargo)))
    return found


def shipped_languages():
    return Catalog("core", "core", [], "core/locales").languages()


def sync(chosen, add):
    for catalog in chosen:
        messages = source_messages(catalog.files, os.path.join(ROOT, catalog.base))
        languages = set(catalog.languages()) | set(add)
        if messages and catalog.base.startswith(("core", "plugins", "companion", ".")):
            languages |= set(shipped_languages())
        languages = sorted(languages)
        if not messages and not catalog.languages():
            continue
        for lang in languages:
            path = os.path.join(ROOT, catalog.locales, f"{lang}.po")
            write_po(path, f"Lounge, {catalog.name}: {lang}", lang, messages, read_po(path))
        print(f"{catalog.name}: {len(messages)} texts in {len(languages)} languages")


def check_catalog(catalog, wanted):
    problems = []
    messages = source_messages(catalog.files, os.path.join(ROOT, catalog.base))
    for message in messages:
        for text in filter(None, [message.text, message.plural]):
            if text != text.strip():
                problems.append(f"{message.files[0]}: \"{text}\" starts or ends with a space; put the space in the code")
    for lang in sorted(set(catalog.languages()) | (set(wanted) if messages else set())):
        path = os.path.join(catalog.locales, f"{lang}.po")
        old = read_po(os.path.join(ROOT, path))
        keys = {m.key for m in messages}
        for key in old:
            if key not in keys:
                problems.append(f"{path}: \"{key[1]}\" is no longer in the source; run tools/words.py sync")
        for message in messages:
            entry = old.get(message.key)
            if entry is None:
                problems.append(f"{path}: \"{message.text}\" is missing; run tools/words.py sync")
                continue
            if entry["plural"] != message.plural:
                problems.append(f"{path}: \"{message.text}\" changed its plural wording; run tools/words.py sync")
            count = plural_count(lang) if message.plural is not None else 1
            if entry["fuzzy"] or len(entry["forms"]) != count or not all(entry["forms"]):
                problems.append(f"{path}: \"{message.text}\" is not translated")
                continue
            places = set(PLACE.findall(message.text + (message.plural or "")))
            for index, form in enumerate(entry["forms"]):
                said = set(PLACE.findall(form))
                # A form for exactly one may spell the number out.
                missing = places - said - ({"{n}"} if message.plural is not None and index == 0 and count > 1 else set())
                if missing or said - places:
                    problems.append(f"{path}: \"{form}\" should fill the same places as \"{message.text}\": {' '.join(sorted(places))}")
                if form != form.strip():
                    problems.append(f"{path}: \"{form}\" starts or ends with a space")
    return problems, len(messages)


# ----- Android resources -----

ANDROID_PLACE = re.compile(r"%(?:\d+\$)?[sd]")


def android_strings(directory):
    """Translatable resources under one values directory: name to text, or to {quantity: text}."""
    found = {}
    for path in sorted(glob.glob(os.path.join(directory, "*.xml"))):
        try:
            root = ET.parse(path).getroot()
        except ET.ParseError as e:
            sys.exit(f"{os.path.relpath(path, ROOT)}: {e}")
        for node in root:
            if node.get("translatable") == "false":
                continue
            if node.tag == "string":
                found[node.get("name")] = "".join(node.itertext())
            elif node.tag == "plurals":
                found[node.get("name")] = {item.get("quantity"): "".join(item.itertext()) for item in node}
            elif node.tag == "string-array":
                found[node.get("name")] = {str(i): "".join(item.itertext()) for i, item in enumerate(node)}
    return found


def check_android(wanted):
    problems, total = [], 0
    qualifiers = {"pt-BR": "pt", "pt-PT": "pt-rPT"}
    for values in sorted(glob.glob(os.path.join(ROOT, "android/**/src/main/res/values"), recursive=True)):
        source = android_strings(values)
        if not source:
            continue
        total += len(source)
        res = os.path.dirname(values)
        for name, text in source.items():
            for said in ([text] if isinstance(text, str) else text.values()):
                if "%s" in said or "%d" in said:
                    problems.append(f"{os.path.relpath(values, ROOT)}: {name} should number its places (%1$s), so a translation can reorder them")
        for lang in wanted:
            qualifier = qualifiers.get(lang, lang.replace("-", "-r"))
            path = os.path.relpath(os.path.join(res, f"values-{qualifier}"), ROOT)
            theirs = android_strings(os.path.join(res, f"values-{qualifier}"))
            for name in theirs.keys() - source.keys():
                problems.append(f"{path}: {name} is not in the English resources")
            for name, text in source.items():
                said = theirs.get(name)
                if said is None or isinstance(said, str) != isinstance(text, str):
                    problems.append(f"{path}: {name} is not translated")
                    continue
                if isinstance(text, str):
                    pairs = [(text, said)]
                else:
                    quantities = ANDROID_QUANTITIES.get(lang.split("-")[0], ["one", "other"]) if "other" in text else list(text)
                    for quantity in quantities:
                        if quantity not in said:
                            problems.append(f"{path}: {name} has no wording for \"{quantity}\"")
                    pairs = [(text.get("other", next(iter(text.values()))), form) for form in said.values()]
                for english, theirs_said in pairs:
                    if not theirs_said.strip():
                        problems.append(f"{path}: {name} is empty")
                    wanted_places, got = sorted(ANDROID_PLACE.findall(english)), sorted(ANDROID_PLACE.findall(theirs_said))
                    if got != wanted_places and not (set(got) < set(wanted_places) and not isinstance(text, str)):
                        problems.append(f"{path}: {name} should fill {' '.join(wanted_places) or 'no places'}, not {' '.join(got) or 'none'}")
    return problems, total


def main(args):
    if not args or args[0] not in ("check", "sync"):
        sys.exit(__doc__)
    add = []
    while "--add" in args:
        at = args.index("--add")
        add.append(args[at + 1])
        del args[at:at + 2]
    named = {"core": 0, "sdk": 1, "phone": 2}
    own = [catalogs()[named[d]] if d in named else plugin_catalog(d) for d in args[1:]]
    chosen = own or catalogs()
    if args[0] == "sync":
        return sync(chosen, add)
    wanted = shipped_languages()
    problems, counted = [], []
    for catalog in chosen:
        found, count = check_catalog(catalog, [] if own else wanted)
        problems += found
        counted.append(count)
    if not own:
        found, count = check_android(wanted)
        problems += found
        counted.append(count)
    for problem in problems:
        print(problem)
    if problems:
        sys.exit(f"\n{len(problems)} problems in the translations")
    print(f"{sum(counted)} texts, complete in {', '.join(wanted) if not own else 'every language the plugin ships'}")


if __name__ == "__main__":
    main(sys.argv[1:])
