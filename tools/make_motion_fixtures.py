"""Build synthetic, unencrypted motion fixtures without proprietary assets."""
from collections import deque
from pathlib import Path
import json
import struct
import argparse

def build(root, version=2):
    keys = set()
    def names(value):
        if isinstance(value, dict) and '$bytes' not in value:
            keys.update(value)
            for child in value.values(): names(child)
        elif isinstance(value, list):
            for child in value: names(child)
    names(root)
    keys = {key: index for index, key in enumerate(sorted(keys))}
    trie = {}
    for key, index in keys.items():
        node = trie
        for char in key.encode('utf-8'): node = node.setdefault(char, {})
        node[0] = index
    bases = [0]
    parents = [0xffffffff]
    occupied = {0}
    leaves = [0] * len(keys)
    pending = deque([(0, trie)])
    while pending:
        index, node = pending.popleft()
        base = 1
        while any(base + char in occupied for char in node): base += 1
        bases[index] = base
        for char, child in node.items():
            slot = base + char
            occupied.add(slot)
            if slot >= len(bases):
                bases.extend([0] * (slot + 1 - len(bases)))
                parents.extend([0xffffffff] * (slot + 1 - len(parents)))
            parents[slot] = index
            if char == 0:
                bases[slot] = child
                leaves[child] = index
            else: pending.append((slot, child))
    def array(values):
        width = max(1, (max(values, default=0).bit_length() + 7) // 8)
        count = max(1, (len(values).bit_length() + 7) // 8)
        return bytes([12 + count]) + len(values).to_bytes(count, 'little') + bytes([12 + width]) + b''.join(n.to_bytes(width, 'little') for n in values)
    strings = {}
    resources = []
    def encode(value):
        if value is None: return b'\x01'
        if value is True: return b'\x02'
        if value is False: return b'\x03'
        if isinstance(value, int):
            if value == 0: return b'\x04'
            width = next(n for n in range(1, 9) if -(1 << (8*n-1)) <= value < (1 << (8*n-1)))
            return bytes([4 + width]) + value.to_bytes(width, 'little', signed=True)
        if isinstance(value, float): return b'\x1e' + struct.pack('<f', value)
        if isinstance(value, str):
            index = strings.setdefault(value, len(strings))
            return b'\x18' + struct.pack('<I', index)
        if isinstance(value, dict) and '$bytes' in value:
            index = len(resources)
            resources.append(bytes(value['$bytes']))
            return b'\x1c' + struct.pack('<I', index)
        if isinstance(value, dict): value = dict(sorted(value.items()))
        items = list(value.values()) if isinstance(value, dict) else value
        content = [encode(child) for child in items]
        offsets = []
        size = 0
        for child in content:
            offsets.append(size)
            size += len(child)
        prefix = b'\x21' + array([keys[key] for key in value]) if isinstance(value, dict) else b'\x20'
        return prefix + array(offsets) + b''.join(content)
    content = encode(root)
    trie = array(bases) + array(parents) + (array(leaves) if version >= 3 else b'')
    sections = [trie, array([]), b'', array([]), array([]), content]
    string_data = bytearray()
    string_offsets = []
    for string in strings:
        string_offsets.append(len(string_data))
        string_data.extend(string.encode('utf-8') + b'\0')
    sections[1] = array(string_offsets)
    sections[2] = string_data
    offsets = []
    size = 0
    for data in resources:
        offsets.append(size)
        size += len(data)
    sections[3] = array(offsets)
    sections[4] = array([len(data) for data in resources])
    positions = []
    header_size = {2: 40, 3: 44, 4: 56}[version]
    size = header_size
    for section in sections:
        positions.append(size)
        size += len(section)
    header = [header_size, *positions[:5], size, positions[5]]
    return b'PSB\0' + struct.pack('<HH8I', version, 0, *header) + bytes(header_size - 40) + b''.join(sections) + b''.join(resources)

if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--version', type=int, choices=[2,3,4], default=2)
    args = parser.parse_args()
    args.output.write_bytes(build(json.loads(args.source.read_text()), args.version))
