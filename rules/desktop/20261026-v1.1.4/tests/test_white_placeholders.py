import hashlib
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib


SCRIPT = Path(__file__).resolve().parents[1] / 'scripts' / 'make-white-placeholders.py'


def chunk(kind, data):
    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))


def independent_png(size, *, rgb=(255, 255, 255), method=0, metadata=False, color=2):
    rows = []
    for y in range(size):
        original = bytes(rgb) * size
        encoded = bytearray(original)
        if method == 1:
            for x in range(len(original)):
                encoded[x] = (original[x] - (original[x - 3] if x >= 3 else 0)) % 256
        elif method == 2 and y:
            encoded = bytearray(len(original))
        elif method in (3, 4):
            previous = original if y else bytes(len(original))
            for x in range(len(original)):
                left, up, corner = (original[x - 3] if x >= 3 else 0), previous[x], (previous[x - 3] if x >= 3 else 0)
                if method == 3:
                    predictor = (left + up) // 2
                else:
                    guess = left + up - corner
                    predictor = min((left, up, corner), key=lambda candidate: abs(guess - candidate))
                encoded[x] = (original[x] - predictor) % 256
        rows.append(bytes([method]) + encoded)
    compressed = zlib.compress(b''.join(rows), level=1)
    body = chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, color, 0, 0, 0))
    if metadata:
        body += chunk(b'tEXt', b'Encoder\x00independent-test')
    midpoint = len(compressed) // 2
    return (b'\x89PNG\r\n\x1a\n' + body + chunk(b'IDAT', compressed[:midpoint])
            + chunk(b'IDAT', compressed[midpoint:]) + chunk(b'IEND', b''))


class WhitePlaceholderTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.index = self.root / 'assets.json'
        self.assets = [
            {'id': 'A0009', 'assetRevision': 2, 'name': '受伤守卫', 'type': 'CH',
             'token': '受伤守卫CH', 'filename': '角色-受伤守卫.png', 'placeholderOrdinal': 7},
            {'id': 'A0001', 'assetRevision': 1, 'name': '门厅', 'type': 'SC',
             'token': '门厅SC', 'filename': '场景-门厅.png', 'placeholderOrdinal': 0},
        ]

    def write_index(self, assets=None):
        self.index.write_text(json.dumps({'version': 'lingmo-v7-assets-v1', 'assets': self.assets if assets is None else assets}), encoding='utf-8')

    def cli(self, directory, *, verify=False, ok=True):
        p = subprocess.run([sys.executable, '-X', 'utf8', str(SCRIPT), '--index', str(self.index), '--output-dir', str(directory), *(['--verify'] if verify else [])], capture_output=True, text=True, encoding='utf-8')
        self.assertEqual(p.returncode == 0, ok, p.stdout + p.stderr)
        self.assertNotIn('Traceback', p.stderr, p.stderr)
        return json.loads(p.stdout) if p.returncode == 0 else p.stderr

    def generate(self, suffix='images'):
        self.write_index()
        directory = self.root / suffix
        self.cli(directory)
        return directory

    def test_generate_exact_names_rgb_white_and_stable_noncontiguous_sizes(self):
        directory = self.generate()
        self.assertEqual({p.name for p in directory.iterdir()}, {a['filename'] for a in self.assets})
        hashes = set()
        for a in self.assets:
            data = (directory / a['filename']).read_bytes()
            width, height, depth, color, *_ = struct.unpack('>IIBBBBB', data[16:29])
            self.assertEqual((width, height, depth, color), (300 + a['placeholderOrdinal'], 300 + a['placeholderOrdinal'], 8, 2))
            offset, compressed = 8, b''
            while offset < len(data):
                length = struct.unpack('>I', data[offset:offset + 4])[0]
                if data[offset + 4:offset + 8] == b'IDAT':
                    compressed += data[offset + 8:offset + 8 + length]
                offset += 12 + length
            self.assertEqual(zlib.decompress(compressed), (b'\0' + b'\xff' * (width * 3)) * height)
            hashes.add(hashlib.md5(data).hexdigest())
        self.assertEqual(len(hashes), 2)
        report = self.cli(directory, verify=True)
        self.assertEqual((report['files'], report['distinct_md5'], report['image_generation_calls']), (2, 2, 0))
        self.assets.reverse()
        self.write_index()
        self.cli(directory, verify=True)

    def test_existing_output_is_never_overwritten(self):
        directory = self.generate()
        before = {p.name: p.read_bytes() for p in directory.iterdir()}
        self.assertIn('OUTPUT_EXISTS', self.cli(directory, ok=False))
        self.assertEqual(before, {p.name: p.read_bytes() for p in directory.iterdir()})

    def test_empty_image_scope_creates_and_verifies_empty_directory(self):
        self.write_index([])
        directory = self.root / 'voice-only'
        self.assertEqual(self.cli(directory)['files'], 0)
        self.assertEqual(self.cli(directory, verify=True)['distinct_md5'], 0)
        self.assertEqual(list(directory.iterdir()), [])

    def test_largest_stable_ordinal_generates_and_decodes_6000_square(self):
        self.write_index([dict(self.assets[0], placeholderOrdinal=5700)])
        directory = self.root / 'largest'
        self.assertEqual(self.cli(directory)['square_sizes'], [6000, 6000])
        self.assertEqual(self.cli(directory, verify=True)['square_sizes'], [6000, 6000])

    def test_catalog_rejects_unsafe_names_mismatched_tokens_files_ids_and_ordinals(self):
        changes = [('name', '无'), ('name', '门/厅'), ('filename', '../门厅.png'),
                   ('token', '另一场景SC'), ('placeholderOrdinal', -1), ('placeholderOrdinal', 5701),
                   ('placeholderOrdinal', True), ('assetRevision', 0), ('id', ''), ('type', [])]
        for field, value in changes:
            with self.subTest(field=field, value=value):
                assets = json.loads(json.dumps(self.assets))
                assets[0][field] = value
                self.write_index(assets)
                directory = self.root / 'invalid'
                self.assertIn('CATALOG_', self.cli(directory, ok=False))
                self.assertFalse(directory.exists())
        for field in ['id', 'placeholderOrdinal']:
            with self.subTest(duplicate=field):
                assets = json.loads(json.dumps(self.assets)); assets[1][field] = assets[0][field]
                self.write_index(assets)
                self.assertIn('CATALOG_', self.cli(self.root / 'invalid', ok=False))
        assets = [dict(self.assets[0], name=n, type='PR', token=n+'PR', filename='道具-'+n+'.png', id=str(i), placeholderOrdinal=i) for i, n in enumerate(['PhoneA', 'phoneA'])]
        self.write_index(assets)
        self.assertIn('CATALOG_COLLISION', self.cli(self.root / 'invalid', ok=False))

    def test_verify_decodes_reencoded_png_with_filter_and_metadata(self):
        directory = self.generate()
        for method in [1, 2, 3, 4]:
            a = self.assets[0]
            (directory / a['filename']).write_bytes(independent_png(307, method=method, metadata=True))
            self.cli(directory, verify=True)

    def test_verify_rejects_crc_pixel_size_and_png_mode_tampering(self):
        for label, data, error in [
            ('bad-crc', independent_png(307)[:-1] + b'\x00', 'PNG_CRC'),
            ('not-white', independent_png(307, rgb=(254, 255, 255)), 'PNG_PIXEL'),
            ('wrong-size', independent_png(308), 'PNG_SIZE'),
            ('not-rgb', independent_png(307, color=6), 'PNG_MODE'),
            ('trailing', independent_png(307) + b'extra', 'PNG_STRUCTURE'),
            ('truncated', independent_png(307)[:-4], 'PNG_STRUCTURE'),
        ]:
            with self.subTest(label=label):
                directory = self.generate(label)
                (directory / self.assets[0]['filename']).write_bytes(data)
                self.assertIn(error, self.cli(directory, verify=True, ok=False))

    def test_verify_rejects_missing_extra_nested_symlink_and_copied_files(self):
        for label in ['missing', 'extra', 'nested', 'symlink', 'copy']:
            with self.subTest(label=label):
                directory = self.generate(label)
                first, second = (directory / a['filename'] for a in self.assets)
                if label == 'missing':
                    first.unlink()
                elif label == 'extra':
                    (directory / 'manifest.json').write_text('{}')
                elif label == 'nested':
                    (directory / 'nested').mkdir()
                elif label == 'symlink':
                    first.unlink(); first.symlink_to(second)
                else:
                    first.write_bytes(second.read_bytes())
                self.assertIn('PLACEHOLDER_', self.cli(directory, verify=True, ok=False))


if __name__ == '__main__':
    unittest.main()
