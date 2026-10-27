#!/usr/bin/env python3
"""Read-only text and timeline signals; no semantic judgment or draft rewriting."""
import argparse
from collections import defaultdict
from decimal import Decimal
import hashlib
import json
from pathlib import Path
import sys


def require(condition, where, expected):
    if not condition:
        raise ValueError(f'invalid input: {where} requires {expected}')


def identifier(value, where):
    require(isinstance(value, str) and bool(value.strip()), where, 'nonempty string')
    return value


def array(value, where):
    require(isinstance(value, list), where, 'array')
    return value


def reject_constant(_):
    raise ValueError('invalid input: JSON numbers must be finite')


def encode(value):
    """Keep Decimal values as JSON numbers without a binary-float round trip."""
    if isinstance(value, Decimal):
        return str(value)
    if isinstance(value, dict):
        return '{' + ','.join(json.dumps(k) + ':' + encode(v) for k, v in value.items()) + '}'
    if isinstance(value, list):
        return '[' + ','.join(encode(v) for v in value) + ']'
    return json.dumps(value, ensure_ascii=False, allow_nan=False)


def episodes(value):
    require(isinstance(value, dict), 'root', 'object')
    forms = [key for key in ('units', 'draft', 'episodes') if key in value]
    require(len(forms) == 1, 'root', 'exactly one of units, draft, episodes')
    if forms == ['units']:
        return [(None, value)]
    entries = array(value['episodes'], 'episodes') if forms == ['episodes'] else [value]
    result, seen = [], set()
    for index, entry in enumerate(entries):
        where = f'episodes[{index}]'
        require(isinstance(entry, dict), where, 'object')
        number = entry.get('number')
        require(type(number) is int and number not in seen,
                where + '.number', 'unique integer')
        seen.add(number)
        result.append((number, entry.get('draft')))
    return result


def inspect(number, draft):
    where = f'episode[{number}]'
    require(isinstance(draft, dict), where + '.draft', 'object')
    repeated, lines, unit_ids = defaultdict(list), {}, set()
    offset = Decimal(0)
    windows, missing, unit_totals, line_times = [], [], [], {}
    cut_pairs, previous_cut = [], None

    def record_fields(item, coord, path):
        for field in ('lighting', 'transition'):
            if field in item:
                require(isinstance(item[field], str), path + '.' + field, 'string')
                if item[field]:
                    repeated[(field, item[field])].append(dict(coord))

    for ui, unit in enumerate(array(draft.get('units'), where + '.units')):
        path = f'{where}.units[{ui}]'
        require(isinstance(unit, dict), path, 'object')
        uid = identifier(unit.get('id'), path + '.id')
        require(uid not in unit_ids, path + '.id', 'unique unit ID')
        unit_ids.add(uid)
        unit_start = offset
        shot_ids = set()
        if not unit.get('shots'):
            previous_cut = None
        for si, shot in enumerate(array(unit.get('shots'), path + '.shots')):
            spath = f'{path}.shots[{si}]'
            require(isinstance(shot, dict), spath, 'object')
            sid = identifier(shot.get('id'), spath + '.id')
            require(sid not in shot_ids, spath + '.id', 'unique shot ID within unit')
            shot_ids.add(sid)
            duration = shot.get('duration')
            require(type(duration) in (int, Decimal) and duration >= 0,
                    spath + '.duration', 'nonnegative finite number')
            coord = {'unit': uid, 'shot': sid}
            record_fields(shot, coord, spath)
            # Copy evidence only: the reader decides whether this cut is coherent.
            cut = dict(coord, **{key: shot[key] for key in
                       ('duration', 'camera', 'picture', 'lighting', 'sound', 'transition',
                        'dialogueParts', 'sourceRefs', 'contextRefs') if key in shot})
            if isinstance(unit.get('segments'), list):
                cut['segment_headers'] = [part for part in unit['segments']
                                          if isinstance(part, dict)
                                          and isinstance(part.get('shotIds'), list)
                                          and sid in part['shotIds']]
            header = {key: unit[key] for key in
                      ('id', 'eventId', 'sceneId', 'scene', 'boundaryBefore', 'initialState',
                       'exitState', 'segments') if key in unit}
            if previous_cut is not None:
                previous, previous_header = previous_cut
                cross = previous['unit'] != uid
                cut_pairs.append({'outgoing': previous, 'incoming': cut, 'crosses_unit': cross,
                                  'unit_boundary': {'outgoing': previous_header, 'incoming': header}
                                  if cross else None})
            previous_cut = cut, header
            shot_lines = {}
            for pi, part in enumerate(array(shot.get('dialogueParts', []), spath + '.dialogueParts')):
                ppath = f'{spath}.dialogueParts[{pi}]'
                require(isinstance(part, dict), ppath, 'object')
                lid = identifier(part.get('lineId'), ppath + '.lineId')
                require(isinstance(part.get('text'), str), ppath + '.text', 'string')
                shot_lines.setdefault(lid, []).append(part['text'])
                voice_coord = dict(coord, lineId=lid)
                a, b = part.get('voiceStart'), part.get('voiceEnd')
                if a is None or b is None:
                    missing.append(voice_coord)
                else:
                    require(type(a) in (int, Decimal) and type(b) in (int, Decimal)
                            and 0 <= a < b <= duration,
                            ppath, '0 <= voiceStart < voiceEnd <= duration')
                    start, end = offset + a, offset + b
                    windows.append((start, end, voice_coord))
                    if lid not in line_times:
                        line_times[lid] = {'lineId': lid, 'start': start, 'end': end,
                                           'voice_seconds': Decimal(0)}
                    line_times[lid]['start'] = min(line_times[lid]['start'], start)
                    line_times[lid]['end'] = max(line_times[lid]['end'], end)
                    line_times[lid]['voice_seconds'] += b - a
            for lid, parts in shot_lines.items():
                lines.setdefault(lid, []).append(dict(coord, duration=duration, text=''.join(parts)))
            offset += duration
        unit_totals.append({'unit': uid, 'start': unit_start, 'end': offset,
                            'seconds': offset - unit_start,
                            'boundary': unit.get('boundaryBefore', {}).get('kind')})

    # Union windows on one episode clock. Overlapping speakers are not added twice.
    end, previous, known_voice = Decimal(0), None, Decimal(0)
    gaps = []
    for start, stop, coord in sorted(windows, key=lambda item: (item[0], item[1])):
        if start > end:
            gaps.append({'start': end, 'end': start, 'seconds': start - end,
                         'previous': previous, 'next': coord})
        known_voice += max(Decimal(0), stop - max(start, end))
        if stop > end:
            end, previous = stop, coord
    if end < offset:
        gaps.append({'start': end, 'end': offset, 'seconds': offset - end,
                     'previous': previous, 'next': None})
    ordered_lines = sorted(line_times.values(), key=lambda item: item['start'])
    adjacent = [{'previous': a['lineId'], 'next': b['lineId'],
                 'seconds': b['start'] - a['end']}
                for a, b in zip(ordered_lines, ordered_lines[1:])]
    timeline = {'total_seconds': offset, 'known_voice_seconds': known_voice,
                'voice_seconds': None if missing else known_voice,
                'outside_voice_seconds': None if missing else offset - known_voice,
                'missing_voice_windows': missing, 'unit_totals': unit_totals,
                'line_timelines': ordered_lines,
                'adjacent_line_intervals': [] if missing else adjacent,
                'longest_outside_voice_spans': [] if missing else
                    sorted(gaps, key=lambda item: (-item['seconds'], item['start']))[:5]}
    return {'number': number,
            'timeline': timeline,
            'cut_pairs': cut_pairs,
            'repeated_text': [{'field': field, 'text': text, 'count': len(coords),
                               'occurrences': coords}
                              for (field, text), coords in repeated.items() if len(coords) >= 3],
            'dialogue_spans': [{'lineId': lid, 'shot_count': len(shots), 'shots': shots,
                                'text': ''.join(shot['text'] for shot in shots)}
                               for lid, shots in lines.items() if len(shots) >= 3]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    try:
        raw = args.input.read_bytes()
        value = json.loads(raw, parse_float=Decimal, parse_constant=reject_constant)
        reports = [inspect(number, draft) for number, draft in episodes(value)]
        report = {'version': 'editing-signals-v3', 'input_sha256': hashlib.sha256(raw).hexdigest(),
                  'advisory_only': True,
                  'limitations': '仅供内容复核定位，不能替代内容复核；有意重复、稳定光向及跨镜连续对白可以合理。'
                                 '统计完全相同文本、同 lineId 跨镜分布及估排声窗的共同时间线，不推断语义、不判质量。'
                                 '声窗外仍可有动作、读屏、环境声和有效情绪，不等于静音或可删时间。'
                                 '跨单元间隔可能包含真实换场；相邻原句间隔为负表示声窗重叠，不自动判错。'
                                 '声窗缺失时只报已知窗口，不将未知时间算成完整无声段；逐句声窗合计不消除该句片段重叠。'
                                 'cut_pairs逐对保留实际镜文与跨单元边界，不推断动作匹配、出画去向、同场或必须合并；'
                                 '空单元不跨越配对，分段资料仅保留原值；须据源和完整稿复核，不自动补写接点。'
                                 '拼接文本仅为输入片段，不证明已还原源句；无信号也不表示无问题。',
                  'episodes': reports}
        payload = (encode(report) + '\n').encode('utf-8')
        with args.output.open('xb') as stream:
            stream.write(payload)
        print(json.dumps({'output': str(args.output.resolve()), 'input_sha256': report['input_sha256'],
                          'output_sha256': hashlib.sha256(payload).hexdigest(),
                          'episode_count': len(reports),
                          'repeated_text_groups': sum(len(r['repeated_text']) for r in reports),
                          'dialogue_span_groups': sum(len(r['dialogue_spans']) for r in reports)}))
        return 0
    except (OSError, ValueError, UnicodeError, RecursionError) as error:
        print(f'inspect_editing: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
