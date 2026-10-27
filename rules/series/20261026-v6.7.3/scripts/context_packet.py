#!/usr/bin/env python3
"""Freeze manually selected, exact worker inputs without model calls or summaries.

Python 3.9+, standard library only. maxBytes bounds the entire serialized file;
payloadBytes counts the canonical UTF-8 JSON payload, not tokens. kindBytes sums
serialized payload entries (array brackets/commas belong only to payloadBytes).
Whole-file inputs preserve UTF-8 text including newlines; JSON Pointer inputs
preserve JSON values, including decimal precision, but not original formatting.
Completeness, correct episode selection and PRELIMINARY/actual state assessment
remain the upstream reviewer's responsibility. Hashes detect drift, not authorship.
"""
import argparse
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import re
import sys


REQUEST_VERSION = 'series-context-request-v1'
PACKET_VERSION = 'series-context-packet-v1'
KINDS = {'source', 'direction', 'state', 'assets', 'entry', 'draft',
         'plan', 'selection', 'issues', 'rules'}
REQUIRED = {'source', 'direction', 'state', 'assets', 'entry'}
DEFAULT_MAX_BYTES = 131072


class PacketError(ValueError):
    pass


def sha256(raw):
    return hashlib.sha256(raw).hexdigest()


def canonical(value):
    """Deterministic JSON with exact decimal values, including large exponents."""
    def encode(item):
        if isinstance(item, dict):
            return '{' + ','.join(encode(key) + ':' + encode(item[key])
                                  for key in sorted(item)) + '}'
        if isinstance(item, list):
            return '[' + ','.join(encode(child) for child in item) + ']'
        if isinstance(item, Decimal):
            if not item.is_finite():
                raise PacketError('non-finite JSON number')
            return str(item)
        return json.dumps(item, ensure_ascii=False, allow_nan=False, separators=(',', ':'))
    try:
        return encode(value).encode('utf-8')
    except (UnicodeError, TypeError, ValueError, RecursionError):
        raise PacketError('value cannot be represented as strict UTF-8 JSON') from None


def parse_json(raw, label):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise PacketError('duplicate JSON key')
            result[key] = value
        return result

    def invalid_constant(value):
        raise PacketError('non-finite JSON number')

    try:
        return json.loads(raw.decode('utf-8'), object_pairs_hook=unique,
                          parse_float=Decimal, parse_constant=invalid_constant)
    except (ValueError, UnicodeError, RecursionError):
        # JSON diagnostics can echo source/dialogue; report the location only.
        raise PacketError('invalid strict UTF-8 JSON: ' + label) from None


def job_path(workdir):
    root = Path(workdir).resolve(strict=True)
    if not root.is_dir():
        raise PacketError('workdir must be a directory')
    return root


def inside(root, relative):
    if not isinstance(relative, str) or not relative.strip() or '\x00' in relative:
        raise PacketError('path must be a nonempty JOB-relative path')
    path = Path(relative)
    if path.is_absolute() or '..' in path.parts or not path.parts:
        raise PacketError('path must stay inside JOB: ' + relative)
    resolved = (root / path).resolve()
    try:
        resolved.relative_to(root)
    except ValueError:
        raise PacketError('symlink/path escapes JOB: ' + relative) from None
    if resolved == root:
        raise PacketError('path must identify a file inside JOB')
    return resolved


def read_file(root, relative):
    path = inside(root, relative)
    if not path.is_file():
        raise PacketError('input must be an existing regular file: ' + relative)
    return path.read_bytes()


def fields(value, required, optional, label):
    if not isinstance(value, dict) or not required <= set(value) or set(value) - required - optional:
        raise PacketError('invalid fields: ' + label)


def validate_request(request):
    fields(request, {'version', 'episode', 'role', 'inputs'}, set(), 'manifest')
    if request['version'] != REQUEST_VERSION:
        raise PacketError('unsupported manifest version')
    if type(request['episode']) is not int:
        raise PacketError('episode must be an integer (not bool)')
    if request['role'] not in ('author', 'reviewer'):
        raise PacketError('role must be author or reviewer')
    if not isinstance(request['inputs'], list) or not request['inputs']:
        raise PacketError('inputs must be a nonempty list')
    present = set()
    for index, item in enumerate(request['inputs']):
        fields(item, {'kind', 'path', 'sha256'}, {'pointer'}, 'input ' + str(index))
        if not isinstance(item['kind'], str) or item['kind'] not in KINDS:
            raise PacketError('unsupported input kind (history/chat/transcript prohibited)')
        if not isinstance(item['sha256'], str) or not re.fullmatch('[0-9a-f]{64}', item['sha256']):
            raise PacketError('input requires lowercase SHA-256 sha256')
        if 'pointer' in item and not isinstance(item['pointer'], str):
            raise PacketError('pointer must be an RFC 6901 JSON Pointer string')
        present.add(item['kind'])
    required = REQUIRED | ({'draft'} if request['role'] == 'reviewer' else set())
    if not required <= present:
        raise PacketError('missing input kinds: ' + ', '.join(sorted(required - present)))


def project(value, pointer):
    if pointer == '':
        return value
    if not pointer.startswith('/'):
        raise PacketError('pointer must be empty or start with /; URI fragments unsupported')
    for token in pointer[1:].split('/'):
        if re.search('~(?![01])', token):
            raise PacketError('invalid JSON Pointer escape')
        key = token.replace('~1', '/').replace('~0', '~')
        if isinstance(value, dict):
            if key not in value:
                raise PacketError('JSON Pointer object key not found')
            value = value[key]
        elif isinstance(value, list):
            if not re.fullmatch('0|[1-9][0-9]*', key):
                raise PacketError('JSON Pointer array index must be a canonical nonnegative integer')
            if len(key) > len(str(len(value))) or int(key) >= len(value):
                raise PacketError('JSON Pointer array index out of bounds')
            value = value[int(key)]
        else:
            raise PacketError('JSON Pointer cannot descend into scalar')
    return value


def assemble(root, manifest_path, max_bytes):
    if type(max_bytes) is not int or max_bytes < 1:
        raise PacketError('max-bytes must be a positive integer')
    manifest_raw = read_file(root, manifest_path)
    request = parse_json(manifest_raw, manifest_path)
    validate_request(request)
    # A valid source hash is not authorization to dispatch a later season.
    # Existing stand-alone packet projects without workspace state stay compatible.
    if (root / 'state.json').exists():
        from workspace import load_state
        try:
            state = load_state(root)
        except (ValueError, OSError, KeyError) as error:
            raise PacketError('workspace scope: ' + str(error)) from None
        if state.get('productionScope') is not None and request['episode'] not in state['scope']:
            raise PacketError('episode outside locked season production scope')
    payload = []
    kind_bytes = {}
    for item in request['inputs']:
        raw = read_file(root, item['path'])
        if sha256(raw) != item['sha256']:
            raise PacketError('sha256 mismatch: ' + item['path'])
        entry = dict(item, originBytes=len(raw))
        if 'pointer' in item:
            entry.update(format='json', value=project(parse_json(raw, item['path']), item['pointer']))
        else:
            try:
                entry.update(format='text', value=raw.decode('utf-8'))
            except UnicodeError:
                raise PacketError('input must be UTF-8 text: ' + item['path']) from None
        payload.append(entry)
        kind_bytes[item['kind']] = kind_bytes.get(item['kind'], 0) + len(canonical(entry))
    payload_raw = canonical(payload)
    packet = {'version': PACKET_VERSION, 'episode': request['episode'], 'role': request['role'],
              'requestOrigin': {'path': manifest_path, 'sha256': sha256(manifest_raw),
                                'bytes': len(manifest_raw)},
              'request': request, 'maxBytes': max_bytes, 'payload': payload,
              'payloadBytes': len(payload_raw), 'kindBytes': kind_bytes,
              'payloadSha256': sha256(payload_raw)}
    if (root / 'state.json').exists() and state.get('productionScope') is not None:
        packet['scopeHash'] = state['scopeHash']
    packet['packetSha256'] = sha256(canonical(packet))
    encoded = canonical(packet)
    if len(encoded) > max_bytes:
        sizes = ', '.join(kind + '=' + str(size) for kind, size in sorted(kind_bytes.items()))
        raise PacketError('max-bytes exceeded: packet=' + str(len(encoded)) + ', budget=' +
                          str(max_bytes) + '; kindBytes: ' + sizes + '; no output written')
    return packet, encoded


def build(root, manifest_path, output, max_bytes):
    target = inside(root, output)
    if target.exists() or (root / output).is_symlink():
        raise PacketError('output already exists; refusing overwrite: ' + output)
    packet, encoded = assemble(root, manifest_path, max_bytes)
    target.parent.mkdir(parents=True, exist_ok=True)
    # Recheck after creating directories and use exclusive creation.
    if inside(root, output) != target:
        raise PacketError('output path changed during build')
    with target.open('xb') as stream:
        stream.write(encoded)
    return packet, encoded


def verify(root, packet_path):
    raw = read_file(root, packet_path)
    packet = parse_json(raw, packet_path)
    if not isinstance(packet, dict) or packet.get('version') != PACKET_VERSION:
        raise PacketError('unsupported packet version')
    origin = packet.get('requestOrigin')
    fields(origin, {'path', 'sha256', 'bytes'}, set(), 'requestOrigin')
    if 'maxBytes' not in packet:
        raise PacketError('missing packet maxBytes')
    expected, encoded = assemble(root, origin['path'], packet['maxBytes'])
    # This checks original hashes, exact projections, metadata, and the integrity
    # checksum. Reformatting is also rejected: the packet is a frozen file.
    if raw != encoded:
        raise PacketError('packet mismatch: source, projection, metadata or bytes changed')
    return expected, raw


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    builder = commands.add_parser('build', help='freeze exact manually selected inputs')
    builder.add_argument('--workdir', required=True)
    builder.add_argument('--manifest', required=True, help='JOB-relative request JSON')
    builder.add_argument('--output', required=True, help='new JOB-relative packet JSON')
    builder.add_argument('--max-bytes', type=int, default=DEFAULT_MAX_BYTES,
                         help='maximum entire packet bytes (default: 131072)')
    checker = commands.add_parser('verify', help='compare frozen packet with all original references')
    checker.add_argument('--workdir', required=True)
    checker.add_argument('--packet', required=True, help='JOB-relative packet JSON')
    args = parser.parse_args(argv)
    try:
        root = job_path(args.workdir)
        if args.command == 'build':
            packet, raw = build(root, args.manifest, args.output, args.max_bytes)
            relative = args.output
        else:
            packet, raw = verify(root, args.packet)
            relative = args.packet
        print(json.dumps({'path': str(inside(root, relative)), 'bytes': len(raw),
                          'sha256': sha256(raw), 'inputs': len(packet['payload'])}, ensure_ascii=False))
        return 0
    except (PacketError, OSError, RuntimeError) as error:
        print('context_packet: ' + str(error), file=sys.stderr)
        return 1


if __name__ == '__main__':
    sys.exit(main())
