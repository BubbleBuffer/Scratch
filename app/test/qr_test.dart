// The QR encoder against values from the QR specification (format and
// version information, alignment positions, capacities) and a small
// independent decoder: it reads the format, unmasks, reads the codewords
// back, checks every block's Reed-Solomon syndromes and parses the data.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:notes_app/features/sync/pairing_codec.dart';
import 'package:notes_app/features/sync/qr.dart';

// Format information strings (MSB first) by level and mask, from the spec.
const _formatL = ['111011111000100', '111001011110011', '111110110101010', '111100010011101', '110011000101111', '110001100011000', '110110001000001', '110100101110110'];
const _formatM = ['101010000010010', '101000100100101', '101111001111100', '101101101001011', '100010111111001', '100000011001110', '100111110010111', '100101010100000'];

int _gfMul(int x, int y) {
  var r = 0;
  while (y > 0) {
    if (y & 1 != 0) r ^= x;
    x <<= 1;
    if (x & 0x100 != 0) x ^= 0x11D;
    y >>= 1;
  }
  return r;
}

int _gfPow(int x, int n) {
  var r = 1;
  for (var i = 0; i < n; i++) {
    r = _gfMul(r, x);
  }
  return r;
}

/// Reads format bits 14..0 from the first copy (around the top-left finder).
String _formatOf(QrCode q) {
  final bits = List.filled(15, 0);
  for (var i = 0; i <= 5; i++) {
    bits[i] = q.dark(8, i) ? 1 : 0;
  }
  bits[6] = q.dark(8, 7) ? 1 : 0;
  bits[7] = q.dark(8, 8) ? 1 : 0;
  bits[8] = q.dark(7, 8) ? 1 : 0;
  for (var i = 9; i < 15; i++) {
    bits[i] = q.dark(14 - i, 8) ? 1 : 0;
  }
  return [for (var i = 14; i >= 0; i--) bits[i]].join();
}

/// Format bits 14..0 from the second copy (bottom-left and top-right).
String _formatOf2(QrCode q) {
  final n = q.size;
  final bits = List.filled(15, 0);
  for (var i = 0; i < 8; i++) {
    bits[i] = q.dark(n - 1 - i, 8) ? 1 : 0;
  }
  for (var i = 8; i < 15; i++) {
    bits[i] = q.dark(8, n - 15 + i) ? 1 : 0;
  }
  return [for (var i = 14; i >= 0; i--) bits[i]].join();
}

bool _maskAt(int mask, int x, int y) => switch (mask) {
      0 => (x + y) % 2 == 0,
      1 => y % 2 == 0,
      2 => x % 3 == 0,
      3 => (x + y) % 3 == 0,
      4 => (x ~/ 3 + y ~/ 2) % 2 == 0,
      5 => (x * y) % 2 + (x * y) % 3 == 0,
      6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
      _ => ((x + y) % 2 + (x * y) % 3) % 2 == 0,
    };

/// Decodes a byte-mode code; fails the test on any inconsistency.
String _decode(QrCode q, QrEcc ecc) {
  final formats = ecc == QrEcc.low ? _formatL : _formatM;
  final f = _formatOf(q);
  final mask = formats.indexOf(f);
  expect(mask, greaterThanOrEqualTo(0), reason: 'format bits $f');
  expect(_formatOf2(q), f);
  expect(q.dark(8, q.size - 8), isTrue, reason: 'dark module');

  // Read the data modules in the zigzag order, unmasked.
  final n = q.size;
  final bits = <int>[];
  for (var right = n - 1; right >= 1; right -= 2) {
    if (right == 6) right = 5;
    for (var vert = 0; vert < n; vert++) {
      for (var j = 0; j < 2; j++) {
        final x = right - j;
        final up = ((right + 1) & 2) == 0;
        final y = up ? n - 1 - vert : vert;
        if (q.isFunction(x, y)) continue;
        bits.add((q.dark(x, y) != _maskAt(mask, x, y)) ? 1 : 0);
      }
    }
  }
  final (numBlocks, eccLen, raw) = QrCode.layout(q.version, ecc);
  final codewords = [for (var i = 0; i + 8 <= bits.length && i ~/ 8 < raw; i += 8) bits.sublist(i, i + 8).fold(0, (a, b) => a << 1 | b)];
  expect(codewords.length, raw);

  // De-interleave.
  final numShort = numBlocks - raw % numBlocks;
  final shortLen = raw ~/ numBlocks;
  final blocks = [for (var i = 0; i < numBlocks; i++) <int>[]];
  var k = 0;
  final dataLen = [for (var i = 0; i < numBlocks; i++) shortLen - eccLen + (i < numShort ? 0 : 1)];
  final longest = dataLen.reduce((a, b) => a > b ? a : b);
  for (var i = 0; i < longest; i++) {
    for (var j = 0; j < numBlocks; j++) {
      if (i < dataLen[j]) blocks[j].add(codewords[k++]);
    }
  }
  for (var i = 0; i < eccLen; i++) {
    for (var j = 0; j < numBlocks; j++) {
      blocks[j].add(codewords[k++]);
    }
  }
  expect(k, raw);

  // Every block is a codeword: its polynomial vanishes at a^0..a^(ecc-1).
  final data = <int>[];
  for (var j = 0; j < numBlocks; j++) {
    final b = blocks[j];
    for (var r = 0; r < eccLen; r++) {
      final root = _gfPow(2, r);
      var s = 0;
      for (final c in b) {
        s = _gfMul(s, root) ^ c;
      }
      expect(s, 0, reason: 'block $j syndrome $r');
    }
    data.addAll(b.sublist(0, dataLen[j]));
  }

  // Parse: byte mode, count, bytes.
  final dbits = [for (final d in data) for (var i = 7; i >= 0; i--) (d >> i) & 1];
  var p = 0;
  int take(int w) {
    var v = 0;
    for (var i = 0; i < w; i++) {
      v = v << 1 | dbits[p++];
    }
    return v;
  }

  expect(take(4), 4, reason: 'byte mode');
  final count = take(q.version < 10 ? 8 : 16);
  final bytes = [for (var i = 0; i < count; i++) take(8)];
  return utf8.decode(bytes);
}

void main() {
  test('alignment pattern positions match the spec table', () {
    expect(QrCode.alignmentPositions(1), isEmpty);
    expect(QrCode.alignmentPositions(2), [6, 18]);
    expect(QrCode.alignmentPositions(7), [6, 22, 38]);
    expect(QrCode.alignmentPositions(8), [6, 24, 42]);
    expect(QrCode.alignmentPositions(14), [6, 26, 46, 66]);
    expect(QrCode.alignmentPositions(32), [6, 34, 60, 86, 112, 138]);
    expect(QrCode.alignmentPositions(40), [6, 30, 58, 86, 114, 142, 170]);
  });

  test('byte capacities match the spec table', () {
    const m = {1: 14, 2: 26, 3: 42, 4: 62, 5: 84, 6: 106, 7: 122, 8: 152, 9: 180, 10: 213, 20: 666, 40: 2331};
    const l = {1: 17, 5: 106, 10: 271, 40: 2953};
    m.forEach((v, c) => expect(QrCode.byteCapacity(v, QrEcc.medium), c, reason: 'M v$v'));
    l.forEach((v, c) => expect(QrCode.byteCapacity(v, QrEcc.low), c, reason: 'L v$v'));
    expect(QrCode.encode('a' * 122).version, 7);
    expect(QrCode.encode('a' * 123).version, 8);
  });

  test('version information of version 7 is 0x07C94', () {
    final q = QrCode.encode('a' * 110);
    expect(q.version, 7);
    var bits = 0, bits2 = 0;
    for (var i = 0; i < 18; i++) {
      final a = q.size - 11 + i % 3, b = i ~/ 3;
      if (q.dark(a, b)) bits |= 1 << i;
      if (q.dark(b, a)) bits2 |= 1 << i;
    }
    expect(bits, 0x07C94);
    expect(bits2, 0x07C94);
  });

  test('every mask decodes back to the text', () {
    const text = 'notes://pair?hub=http://100.101.102.103:47100&token=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';
    for (var mask = 0; mask < 8; mask++) {
      final q = QrCode.encode(text, forceMask: mask);
      expect(q.mask, mask);
      expect(_decode(q, QrEcc.medium), text, reason: 'mask $mask');
    }
  });

  test('many lengths and both levels decode back', () {
    for (final ecc in [QrEcc.low, QrEcc.medium]) {
      for (final len in [0, 1, 13, 14, 15, 60, 150, 250, 400, 1000]) {
        final text = String.fromCharCodes([for (var i = 0; i < len; i++) 33 + (i * 7) % 90]);
        final q = QrCode.encode(text, ecc: ecc);
        expect(_decode(q, ecc), text, reason: '${ecc.name} len $len v${q.version}');
      }
    }
    expect(_decode(QrCode.encode('Grüße 🌱'), QrEcc.medium), 'Grüße 🌱');
  });

  test('pairing strings round-trip', () {
    const token = '0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef';
    final p = Pairing('http://100.101.102.103:47100', token);
    final back = Pairing.parse(p.encode())!;
    expect(back.url, p.url);
    expect(back.token, token);
    final plain = Pairing.parse('192.168.0.5:47100 $token')!;
    expect(plain.url, 'http://192.168.0.5:47100');
    expect(Pairing.parse('hello'), isNull);
    expect(normalizeHubUrl('100.101.102.103:47100/'), 'http://100.101.102.103:47100');
    expect(portOf('0.0.0.0:47100'), 47100);
  });
}
