#!/usr/bin/env python3
"""Generate or independently verify pure-white RGB PNGs from a lingmo-v7 catalog."""

import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys
import unicodedata
import zlib


PREFIX = {'CH': '角色', 'SC': '场景', 'PR': '道具'}
SIGNATURE = b'\x89PNG\r\n\x1a\n'


def require(ok, code, message):
    if not ok:
        raise ValueError('%s: %s' % (code, message))


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def load_catalog(path):
    catalog = json.loads(path.read_text(encoding='utf-8'))
    require(isinstance(catalog, dict) and catalog.get('version') == 'lingmo-v7-assets-v1', 'CATALOG_VERSION', '需要 lingmo-v7-assets-v1 导出目录')
    assets = catalog.get('assets')
    require(isinstance(assets, list) and len(assets) <= 5701, 'CATALOG_ASSETS', 'assets 须为最多 5701 项的数组；纯声音项目允许空数组')
    ids, ordinals, filenames = set(), set(), set()
    for asset in assets:
        require(isinstance(asset, dict), 'CATALOG_ASSET', '资产记录必须为对象')
        asset_id, name, kind = asset.get('id'), asset.get('name'), asset.get('type')
        require(isinstance(asset_id, str) and asset_id.strip() and not any(c in asset_id for c in '\0\r\n') and asset_id not in ids, 'CATALOG_ID', '资产 ID 为空、重复或无效')
        require(integer(asset.get('assetRevision'), 1, 2**53 - 1), 'CATALOG_REVISION', asset_id + ' 缺少有效资产版本')
        require(isinstance(name, str) and name != '无' and name.isalnum() and isinstance(kind, str) and kind in PREFIX, 'CATALOG_NAME', asset_id + ' 的安全名或类型无效')
        require(asset.get('token') == name + kind, 'CATALOG_TOKEN', asset_id + ' 的令牌不匹配')
        filename = PREFIX[kind] + '-' + name + '.png'
        require(asset.get('filename') == filename and len(filename.encode('utf-8')) <= 255, 'CATALOG_FILENAME', asset_id + ' 的文件名不匹配或过长')
        folded = unicodedata.normalize('NFKC', filename).casefold()
        require(folded not in filenames, 'CATALOG_COLLISION', asset_id + ' 的文件名存在大小写或规范化冲突')
        ordinal = asset.get('placeholderOrdinal')
        require(integer(ordinal, 0, 5700) and ordinal not in ordinals, 'CATALOG_ORDINAL', asset_id + ' 的占位编号无效或重复，允许 0–5700')
        ids.add(asset_id); filenames.add(folded); ordinals.add(ordinal)
    return assets


def png(size):
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    # Dimensions, rather than metadata, carry uniqueness after lossless re-encoding.
    encoder = zlib.compressobj()
    row = b'\0' + b'\xff' * (size * 3)
    compressed = b''.join(encoder.compress(row) for _ in range(size)) + encoder.flush()
    return (SIGNATURE + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', compressed) + chunk(b'IEND', b''))


def paeth(left, up, upper_left):
    p = left + up - upper_left
    distances = (abs(p - left), abs(p - up), abs(p - upper_left))
    return (left, up, upper_left)[distances.index(min(distances))]


def decode_white_png(data, expected_size, filename):
    """Parse chunks, check CRCs, inflate and undo all five PNG row filters."""
    require(data.startswith(SIGNATURE), 'PNG_STRUCTURE', filename + ' 不是 PNG')
    offset, header, ended, saw_idat, idat_closed, saw_palette = 8, None, False, False, False, False
    compressed = bytearray()
    while offset < len(data):
        require(offset + 12 <= len(data), 'PNG_STRUCTURE', filename + ' PNG 块被截断')
        length = struct.unpack('>I', data[offset:offset + 4])[0]
        kind = data[offset + 4:offset + 8]
        stop = offset + 12 + length
        require(stop <= len(data) and all(65 <= c <= 90 or 97 <= c <= 122 for c in kind), 'PNG_STRUCTURE', filename + ' PNG 块长度或类型无效')
        content = data[offset + 8:offset + 8 + length]
        crc = struct.unpack('>I', data[offset + 8 + length:stop])[0]
        require(zlib.crc32(kind + content) == crc, 'PNG_CRC', filename + ' 的 PNG CRC 不匹配')
        require(header is not None or kind == b'IHDR', 'PNG_STRUCTURE', filename + ' 缺少首个 IHDR')
        if kind == b'IHDR':
            require(header is None and length == 13, 'PNG_STRUCTURE', filename + ' IHDR 重复或长度无效')
            header = struct.unpack('>IIBBBBB', content)
            width, height, depth, color, compression, filtering, interlace = header
            require((width, height) == (expected_size, expected_size), 'PNG_SIZE', filename + ' 尺寸与稳定编号不匹配')
            require((depth, color, compression, filtering, interlace) == (8, 2, 0, 0, 0), 'PNG_MODE', filename + ' 必须为 8 位 RGB、无透明度、无交错 PNG')
        elif kind == b'IDAT':
            require(not idat_closed, 'PNG_STRUCTURE', filename + ' 的 IDAT 不连续')
            saw_idat = True
            compressed.extend(content)
        elif kind == b'IEND':
            require(length == 0 and saw_idat and stop == len(data), 'PNG_STRUCTURE', filename + ' 的 IEND 或末尾数据无效')
            ended = True
            offset = stop
            break
        else:
            if saw_idat:
                idat_closed = True
            if kind == b'PLTE':
                require(not saw_palette and not saw_idat and 0 < length <= 768 and length % 3 == 0, 'PNG_STRUCTURE', filename + ' PLTE 无效')
                saw_palette = True
            else:
                require(kind[0] & 32, 'PNG_STRUCTURE', filename + ' 包含未知关键 PNG 块')
                require(kind != b'tRNS', 'PNG_MODE', filename + ' 不能含透明色')
        offset = stop
    require(header is not None and ended and offset == len(data), 'PNG_STRUCTURE', filename + ' 缺少完整 PNG 结束标记')
    row_bytes = expected_size * 3
    expected_bytes = (row_bytes + 1) * expected_size
    decoder = zlib.decompressobj()
    try:
        raw = decoder.decompress(bytes(compressed), expected_bytes + 1)
    except zlib.error as error:
        raise ValueError('PNG_DECODE: %s 解压失败' % filename) from error
    require(len(raw) == expected_bytes and decoder.eof and not decoder.unused_data and not decoder.unconsumed_tail, 'PNG_DECODE', filename + ' 解压长度或压缩流无效')
    white = b'\xff' * row_bytes
    previous = bytearray(row_bytes)
    for y in range(expected_size):
        start = y * (row_bytes + 1)
        method = raw[start]
        require(method <= 4, 'PNG_DECODE', filename + ' 含未知扫描行滤镜')
        row = bytearray(raw[start + 1:start + 1 + row_bytes])
        if method:
            for x in range(row_bytes):
                left, up, upper_left = (row[x - 3] if x >= 3 else 0), previous[x], (previous[x - 3] if x >= 3 else 0)
                predictor = left if method == 1 else up if method == 2 else (left + up) // 2 if method == 3 else paeth(left, up, upper_left)
                row[x] = (row[x] + predictor) & 255
        require(row == white, 'PNG_PIXEL', filename + ' 含非纯白 RGB 像素')
        previous = row


def verify_directory(assets, directory):
    require(directory.is_dir() and not directory.is_symlink(), 'PLACEHOLDER_DIRECTORY', '素材目录不存在或是符号链接')
    entries = list(directory.iterdir())
    expected = {a['filename'] for a in assets}
    require({p.name for p in entries} == expected, 'PLACEHOLDER_FILES', '素材文件名集合不一致，含缺失、额外文件或目录')
    require(all(p.is_file() and not p.is_symlink() and p.suffix == '.png' for p in entries), 'PLACEHOLDER_FILES', '素材目录仅允许扁平 PNG 普通文件')
    hashes, sizes = set(), []
    # Hash uniqueness is a separate upload requirement; pixel decoding below is
    # authoritative for content and is independent of this script's encoder.
    for asset in assets:
        data = (directory / asset['filename']).read_bytes()
        md5 = hashlib.md5(data).hexdigest()
        require(md5 not in hashes, 'PLACEHOLDER_DUPLICATE', '占位图片不能复用相同 MD5 内容')
        hashes.add(md5)
    for asset in assets:
        size = 300 + asset['placeholderOrdinal']
        decode_white_png((directory / asset['filename']).read_bytes(), size, asset['filename'])
        sizes.append(size)
    return {'ok': True, 'files': len(assets), 'distinct_md5': len(hashes),
            'square_sizes': [min(sizes), max(sizes)] if sizes else [], 'image_generation_calls': 0}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--index', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    parser.add_argument('--verify', action='store_true', help='只读并独立解码核验已有 PNG 目录')
    args = parser.parse_args()
    assets = load_catalog(args.index)
    if not args.verify:
        try:
            args.output_dir.mkdir(parents=True, exist_ok=False)
        except FileExistsError as error:
            raise ValueError('OUTPUT_EXISTS: 输出目录已存在，禁止覆盖；新交付使用新目录') from error
        for asset in assets:
            with (args.output_dir / asset['filename']).open('xb') as output:
                output.write(png(300 + asset['placeholderOrdinal']))
    report = verify_directory(assets, args.output_dir)
    report['mode'] = 'verify' if args.verify else 'generate'
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
