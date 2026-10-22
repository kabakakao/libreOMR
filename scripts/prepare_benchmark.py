#!/usr/bin/env python3
"""Download public OMR data and freeze reproducible, disjoint benchmark samples.

Python standard library only. Data stays outside the source tree by default.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import tarfile
import urllib.request
import zipfile
import xml.etree.ElementTree as ET
from collections import defaultdict


SOURCES = {
    "primus": (
        "https://grfia.dlsi.ua.es/primus/packages/primusCalvoRizoAppliedSciences2018.tgz",
        "https://grfia.dlsi.ua.es/primus/",
    ),
    "camera-primus": (
        "https://grfia.dlsi.ua.es/primus/packages/CameraPrIMuS.tgz",
        "https://grfia.dlsi.ua.es/primus/",
    ),
    "doremi": (
        "https://github.com/steinbergmedia/DoReMi/releases/download/v1.0/DoReMi_v1.zip",
        "https://github.com/steinbergmedia/DoReMi/releases/tag/v1.0",
    ),
}


def fingerprint(path):
    # FNV-1a/64: content identity for repeatability, not a security signature.
    value = 0xCBF29CE484222325
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(65536), b""):
            for byte in chunk:
                value = ((value ^ byte) * 0x100000001B3) & 0xFFFFFFFFFFFFFFFF
    return f"{value:016x}"


def group_id(dataset, stem):
    if dataset in ("primus", "camera-primus"):
        # Keep fonts and camera variants of the same RISM incipit together.
        return re.sub(r"[._-](?:bravura|leipzig|gootville)(?:[._-].*)?$", "", stem,
                      flags=re.IGNORECASE)
    # DoReMi page suffixes are -001, -002, ...; split by source score.
    return re.sub(r"-\d+$", "", stem)


def digest(seed, group):
    return hashlib.sha256(f"{seed}:{group}".encode()).hexdigest()


def safe_destination(root, name):
    path = (root / name).resolve()
    if not path.is_relative_to(root.resolve()):
        raise ValueError(f"Archive path escapes destination: {name}")
    return path


def fetch(dataset, cache):
    url = SOURCES[dataset][0]
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / url.rsplit("/", 1)[1]
    if not archive.exists():
        partial = archive.with_suffix(archive.suffix + ".part")
        print(f"Downloading {url}", flush=True)
        with urllib.request.urlopen(url, timeout=120) as source, partial.open("wb") as target:
            total = int(source.headers.get("Content-Length", "0"))
            print(f"Download size: {total / 1024**2:.1f} MiB", flush=True)
            shutil.copyfileobj(source, target)
        if total and partial.stat().st_size != total:
            raise ValueError("Incomplete download; .part retained for inspection")
        partial.replace(archive)
    destination = cache / dataset
    destination.mkdir(exist_ok=True)
    # Only regular files; never follow links from external archives.
    print(f"Extracting into {destination}", flush=True)
    if zipfile.is_zipfile(archive):
        with zipfile.ZipFile(archive) as source:
            for member in source.infolist():
                path = safe_destination(destination, member.filename)
                if member.is_dir():
                    continue
                path.parent.mkdir(parents=True, exist_ok=True)
                with source.open(member) as src, path.open("wb") as dst:
                    shutil.copyfileobj(src, dst)
    else:
        with tarfile.open(archive) as source:
            for member in source:
                path = safe_destination(destination, member.name)
                if not member.isfile():
                    continue
                path.parent.mkdir(parents=True, exist_ok=True)
                with source.extractfile(member) as src, path.open("wb") as dst:
                    shutil.copyfileobj(src, dst)
    print(destination, flush=True)


def prepare(dataset, root, output, limit, seed, test_percent):
    root = root.resolve()
    output = output.resolve()
    images = {}
    for path in sorted(root.rglob("*")):
        if path.suffix.lower() == ".png" and not path.name.startswith("._"):
            images.setdefault(path.stem, []).append(path)
    pairs = []
    extension = ".xml" if dataset == "doremi" else ".semantic"
    for annotation in sorted(root.rglob("*" + extension)):
        if annotation.name.startswith("._"):
            continue
        image_stem = annotation.stem
        if dataset == "doremi":
            parsed = re.fullmatch(r"Parsed_(.*)-layout-0-muscima_Page_(\d+)", image_stem)
            if parsed:
                image_stem = f"{parsed[1]}-{int(parsed[2]):03d}"
        matches = images.get(image_stem, [])
        if not matches:
            continue
        # Prefer sibling files (PrIMuS); DoReMi keeps images and XML separately.
        sibling = annotation.with_suffix(".png")
        if sibling in matches:
            image = sibling
        elif len(matches) == 1:
            image = matches[0]
        else:
            raise ValueError(f"Ambiguous image for {annotation}: {matches}")
        if dataset == "doremi":
            # Exclude MusicXML/MEI files that happen to share a basename.
            with annotation.open("r", encoding="utf-8-sig") as stream:
                if "<Node>" not in stream.read(65536):
                    continue
        group = group_id(dataset, image.stem)
        pairs.append((digest(seed, annotation.relative_to(root).as_posix()), group, image, annotation))
    if not pairs:
        raise ValueError(f"No PNG/{extension} pairs found under {root}")
    # Round-robin through source scores: thousands of synthetic pages from one
    # DoReMi project must not crowd all other projects out of the small sample.
    grouped = defaultdict(list)
    for pair in sorted(pairs):
        grouped[pair[1]].append(pair)
    groups = sorted(grouped, key=lambda group: digest(seed, group))
    selected = []
    depth = 0
    while len(selected) < min(limit, len(pairs)):
        for group in groups:
            if depth < len(grouped[group]):
                selected.append(grouped[group][depth])
                if len(selected) == limit:
                    break
        depth += 1
    enriched = enrich_doremi(selected, root, output) if dataset == "doremi" else {}
    samples = []
    ids = set()
    for _, group, image, annotation in selected:
        identifier = annotation.relative_to(root).with_suffix("").as_posix()
        if identifier in ids:
            raise ValueError(f"Duplicate sample: {identifier}")
        ids.add(identifier)
        split = "test" if int(digest(seed, group)[:8], 16) % 100 < test_percent else "dev"
        annotation = enriched.get(annotation, annotation)
        samples.append({
            "id": identifier, "group": group, "split": split,
            "image": image.relative_to(root).as_posix(),
            "annotation": os.path.relpath(annotation, root),
            "image_fingerprint": fingerprint(image),
            "annotation_fingerprint": fingerprint(annotation),
        })
    output.parent.mkdir(parents=True, exist_ok=True)
    manifest = {
        "version": 1, "dataset": dataset, "source": SOURCES[dataset][1],
        "data_root": os.path.relpath(root, output.parent), "seed": seed,
        "test_percent": test_percent, "candidate_count": len(pairs),
        "samples": samples,
    }
    output.write_text(json.dumps(manifest, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    counts = {split: sum(s["split"] == split for s in samples) for split in ("dev", "test")}
    print(f"{output}: {len(samples)} samples from {len(pairs)} candidates; {counts}")


def enrich_doremi(selected, root, output):
    """Restore musical Data removed by the official per-page XML export.

    Read each original score once, retaining only Data for selected node IDs.
    Generated page XML lives beside the manifest; originals remain untouched.
    """
    by_score = defaultdict(list)
    for _, _, _, annotation in selected:
        parsed = re.fullmatch(r"Parsed_(.*)_Page_\d+", annotation.stem)
        if parsed:
            candidates = list(root.rglob(parsed[1] + ".xml"))
            candidates = [p for p in candidates if "OMR_XML" in p.parts and "__MACOSX" not in p.parts]
            if len(candidates) == 1:
                by_score[candidates[0]].append(annotation)
    result = {}
    destination = output.parent / (output.stem + "-references")
    for original, annotations in sorted(by_score.items()):
        trees = {path: ET.parse(path) for path in annotations}
        wanted = {}
        for tree in trees.values():
            for node in tree.findall(".//Node"):
                wanted[node.findtext("Id")] = node
        for _, node in ET.iterparse(original, events=["end"]):
            if node.tag == "Node":
                target = wanted.get(node.findtext("Id"))
                data = node.find("Data")
                if target is not None and data is not None:
                    # Serialize before clearing the source node.
                    target.append(ET.fromstring(ET.tostring(data)))
                node.clear()
        destination.mkdir(parents=True, exist_ok=True)
        for path, tree in trees.items():
            filename = hashlib.sha256(path.relative_to(root).as_posix().encode()).hexdigest()[:20] + ".xml"
            target = destination / filename
            tree.write(target, encoding="utf-8", xml_declaration=True)
            result[path] = target
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    download = commands.add_parser("fetch", help="Download and extract an official archive")
    download.add_argument("dataset", choices=SOURCES)
    download.add_argument("--cache", type=Path, default=Path("target/omr-data"))
    select = commands.add_parser("prepare", help="Freeze a sample of an extracted dataset")
    select.add_argument("dataset", choices=SOURCES)
    select.add_argument("--input", type=Path, required=True)
    select.add_argument("--output", type=Path, required=True)
    select.add_argument("--limit", type=int, default=500)
    select.add_argument("--seed", type=int, default=42)
    select.add_argument("--test-percent", type=int, default=20)
    args = parser.parse_args()
    if args.command == "fetch":
        fetch(args.dataset, args.cache)
    else:
        if args.limit < 1 or not 1 <= args.test_percent <= 99 or not 0 <= args.seed < 2**64:
            parser.error("--limit must be positive, --test-percent between 1 and 99, and --seed an unsigned 64-bit integer")
        prepare(args.dataset, args.input, args.output, args.limit, args.seed, args.test_percent)


if __name__ == "__main__":
    main()
