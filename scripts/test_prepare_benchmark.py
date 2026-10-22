import json
from pathlib import Path
import tempfile
import unittest
import xml.etree.ElementTree as ET

from prepare_benchmark import fingerprint, group_id, prepare, safe_destination


class PreparationTests(unittest.TestCase):
    def test_grouping_keeps_font_and_page_variants_together(self):
        self.assertEqual(group_id("primus", "123-Bravura"), "123")
        self.assertEqual(group_id("camera-primus", "123-Leipzig-distorted"), "123")
        self.assertEqual(group_id("doremi", "Sonata-001"), "Sonata")
        self.assertEqual(group_id("doremi", "Sonata-1002"), "Sonata")

    def test_reproducible_sampling_and_no_source_leakage(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "data"
            data.mkdir()
            for stem in ["a-Bravura", "a-Leipzig", "b-Bravura", "c-Gootville"]:
                (data / (stem + ".png")).write_bytes(b"hello")
                (data / (stem + ".semantic")).write_text("clef-G2 note-C4_quarter")
            path = root / "sample.json"
            prepare("primus", data, path, 4, 42, 20)
            before = path.read_bytes()
            prepare("primus", data, path, 4, 42, 20)
            self.assertEqual(path.read_bytes(), before)
            samples = json.loads(before)["samples"]
            variants = [s for s in samples if s["group"] == "a"]
            self.assertEqual(len(variants), 2)
            self.assertEqual(variants[0]["split"], variants[1]["split"])
            self.assertEqual(fingerprint(data / "a-Bravura.png"), "a430d84680aabd0b")

    def test_official_doremi_names_and_metadata_enrichment(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            data = root / "data"
            for name in ["Images", "Parsed_by_page_omr_xml", "OMR_XML"]:
                (data / name).mkdir(parents=True)
            (data / "Images/Study-001.png").write_bytes(b"image")
            annotation = "<Page pageIndex='0'><Nodes><Node><Id>12</Id><ClassName>noteheadBlack</ClassName><Top>10</Top><Left>20</Left><Width>8</Width><Height>6</Height></Node></Nodes></Page>"
            (data / "Parsed_by_page_omr_xml/Parsed_Study-layout-0-muscima_Page_1.xml").write_text(annotation)
            original = annotation.replace("</Node>", '<Data><DataItem key="duration_beats">1</DataItem></Data></Node>')
            (data / "OMR_XML/Study-layout-0-muscima.xml").write_text(original)
            output = root / "sample.json"
            prepare("doremi", data, output, 10, 42, 20)
            manifest = json.loads(output.read_text())
            self.assertEqual(len(manifest["samples"]), 1)
            sample = manifest["samples"][0]
            self.assertEqual(sample["group"], "Study")
            enriched = ET.parse(data / sample["annotation"])
            self.assertEqual(enriched.findtext('.//DataItem'), "1")
            self.assertNotIn("DataItem", (data / "Parsed_by_page_omr_xml/Parsed_Study-layout-0-muscima_Page_1.xml").read_text())

    def test_archives_cannot_escape_destination(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(ValueError):
                safe_destination(root, "../outside")
            self.assertEqual(safe_destination(root, "sub/file"), root.resolve() / "sub/file")


if __name__ == "__main__":
    unittest.main()
