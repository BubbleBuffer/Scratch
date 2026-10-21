// A small QR code encoder (byte mode, versions 1-40, any error correction
// level) and a widget that paints the code. Follows ISO/IEC 18004 as laid
// out by Project Nayuki's reference implementation; used for pairing
// strings (about 130 bytes, version 8 at level M).

import 'dart:convert';

import 'package:flutter/widgets.dart';

enum QrEcc {
  low(1),
  medium(0),
  quartile(3),
  high(2);

  const QrEcc(this.formatBits);
  final int formatBits;
}

// Indexed [ecc.index][version]; index 0 unused.
const _eccPerBlock = [
  [-1, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28, 30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
  [-1, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28],
  [-1, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30, 30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
  [-1, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30],
];

const _numBlocks = [
  [-1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13, 14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25],
  [-1, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21, 23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49],
  [-1, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29, 34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68],
  [-1, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32, 35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81],
];

// _eccPerBlock and _numBlocks rows are in L, M, Q, H order.
int _row(QrEcc e) => switch (e) { QrEcc.low => 0, QrEcc.medium => 1, QrEcc.quartile => 2, QrEcc.high => 3 };

class QrCode {
  QrCode._(this.version, this.size, this.mask, this._dark, this._fn);

  final int version;
  final int size;
  final int mask;
  final List<List<bool>> _dark;
  final List<List<bool>> _fn;

  /// Module at column [x], row [y] (true = dark).
  bool dark(int x, int y) => _dark[y][x];

  /// Whether the module is part of a function pattern (finder, timing,
  /// alignment, format or version information), not data.
  bool isFunction(int x, int y) => _fn[y][x];

  /// Row and column centres of the alignment patterns of [version].
  static List<int> alignmentPositions(int version) => _Matrix(version)._alignmentPositions();

  /// Data bytes (byte mode, after mode and length) a version holds.
  static int byteCapacity(int version, QrEcc ecc) =>
      (_numDataCodewords(version, ecc) * 8 - 4 - (version < 10 ? 8 : 16)) ~/ 8;

  /// Data codewords and error correction per block: (blocks, ecc per block,
  /// raw codewords) of a version.
  static (int, int, int) layout(int version, QrEcc ecc) =>
      (_numBlocks[_row(ecc)][version], _eccPerBlock[_row(ecc)][version], _numRawDataModules(version) ~/ 8);

  /// Encodes [text] (UTF-8, byte mode) in the smallest version that fits.
  /// [forceMask] picks the mask (tests); otherwise the lowest penalty wins.
  static QrCode encode(String text, {QrEcc ecc = QrEcc.medium, int? forceMask}) {
    final data = utf8.encode(text);
    var version = 1;
    for (;; version++) {
      if (version > 40) throw ArgumentError('Text too long for a QR code (${data.length} bytes)');
      final cc = version < 10 ? 8 : 16;
      if (4 + cc + data.length * 8 <= _numDataCodewords(version, ecc) * 8) break;
    }
    final bits = <int>[];
    void put(int value, int n) {
      for (var i = n - 1; i >= 0; i--) {
        bits.add((value >> i) & 1);
      }
    }

    put(4, 4); // byte mode
    put(data.length, version < 10 ? 8 : 16);
    for (final b in data) {
      put(b, 8);
    }
    final capacity = _numDataCodewords(version, ecc) * 8;
    put(0, (capacity - bits.length).clamp(0, 4));
    put(0, (8 - bits.length % 8) % 8);
    for (var pad = 0xEC; bits.length < capacity; pad ^= 0xEC ^ 0x11) {
      put(pad, 8);
    }
    final codewords = <int>[
      for (var i = 0; i < bits.length; i += 8)
        bits.sublist(i, i + 8).fold(0, (a, b) => (a << 1) | b),
    ];
    final all = _addEcc(codewords, version, ecc);

    final m = _Matrix(version);
    m.drawFunctionPatterns(ecc);
    m.drawCodewords(all);
    var best = forceMask ?? 0;
    if (forceMask == null) {
      var min = 1 << 30;
      for (var k = 0; k < 8; k++) {
        m.applyMask(k);
        m.drawFormat(ecc, k);
        final p = m.penalty();
        if (p < min) {
          min = p;
          best = k;
        }
        m.applyMask(k); // undo (xor)
      }
    }
    m.applyMask(best);
    m.drawFormat(ecc, best);
    return QrCode._(version, m.size, best, m.dark, m.fn);
  }

  static int _numRawDataModules(int ver) {
    var r = (16 * ver + 128) * ver + 64;
    if (ver >= 2) {
      final n = ver ~/ 7 + 2;
      r -= (25 * n - 10) * n - 55;
      if (ver >= 7) r -= 36;
    }
    return r;
  }

  static int _numDataCodewords(int ver, QrEcc ecc) =>
      _numRawDataModules(ver) ~/ 8 - _eccPerBlock[_row(ecc)][ver] * _numBlocks[_row(ecc)][ver];

  static List<int> _addEcc(List<int> data, int ver, QrEcc ecc) {
    final numBlocks = _numBlocks[_row(ecc)][ver];
    final eccLen = _eccPerBlock[_row(ecc)][ver];
    final raw = _numRawDataModules(ver) ~/ 8;
    final numShort = numBlocks - raw % numBlocks;
    final shortLen = raw ~/ numBlocks;
    final divisor = _rsDivisor(eccLen);
    final blocks = <List<int>>[];
    var k = 0;
    for (var i = 0; i < numBlocks; i++) {
      final len = shortLen - eccLen + (i < numShort ? 0 : 1);
      final dat = data.sublist(k, k + len);
      k += len;
      final e = _rsRemainder(dat, divisor);
      final block = [...dat];
      if (i < numShort) block.add(0);
      block.addAll(e);
      blocks.add(block);
    }
    final out = <int>[];
    for (var i = 0; i < blocks[0].length; i++) {
      for (var j = 0; j < blocks.length; j++) {
        if (i != shortLen - eccLen || j >= numShort) out.add(blocks[j][i]);
      }
    }
    return out;
  }

  static int _mul(int x, int y) {
    var z = 0;
    for (var i = 7; i >= 0; i--) {
      z = (z << 1) ^ ((z >> 7) * 0x11D);
      z ^= ((y >> i) & 1) * x;
    }
    return z & 0xFF;
  }

  static List<int> _rsDivisor(int degree) {
    final r = List.filled(degree, 0);
    r[degree - 1] = 1;
    var root = 1;
    for (var i = 0; i < degree; i++) {
      for (var j = 0; j < r.length; j++) {
        r[j] = _mul(r[j], root);
        if (j + 1 < r.length) r[j] ^= r[j + 1];
      }
      root = _mul(root, 0x02);
    }
    return r;
  }

  static List<int> _rsRemainder(List<int> data, List<int> divisor) {
    final r = List.filled(divisor.length, 0, growable: true);
    for (final b in data) {
      final factor = b ^ r.removeAt(0);
      r.add(0);
      for (var i = 0; i < r.length; i++) {
        r[i] ^= _mul(divisor[i], factor);
      }
    }
    return r;
  }
}

class _Matrix {
  _Matrix(this.version)
      : size = version * 4 + 17,
        dark = List.generate(version * 4 + 17, (_) => List.filled(version * 4 + 17, false)),
        fn = List.generate(version * 4 + 17, (_) => List.filled(version * 4 + 17, false));

  final int version;
  final int size;
  final List<List<bool>> dark;
  final List<List<bool>> fn;

  void set(int x, int y, bool d) {
    dark[y][x] = d;
    fn[y][x] = true;
  }

  List<int> _alignmentPositions() {
    if (version == 1) return const [];
    final n = version ~/ 7 + 2;
    final step = version == 32 ? 26 : ((version * 4 + n * 2 + 1) ~/ (n * 2 - 2)) * 2;
    final r = [6];
    for (var pos = size - 7; r.length < n; pos -= step) {
      r.insert(1, pos);
    }
    return r;
  }

  void drawFunctionPatterns(QrEcc ecc) {
    for (var i = 0; i < size; i++) {
      set(6, i, i % 2 == 0);
      set(i, 6, i % 2 == 0);
    }
    for (final (x, y) in [(3, 3), (size - 4, 3), (3, size - 4)]) {
      for (var dy = -4; dy <= 4; dy++) {
        for (var dx = -4; dx <= 4; dx++) {
          final d = dx.abs() > dy.abs() ? dx.abs() : dy.abs();
          final xx = x + dx, yy = y + dy;
          if (xx >= 0 && xx < size && yy >= 0 && yy < size) set(xx, yy, d != 2 && d != 4);
        }
      }
    }
    final pos = _alignmentPositions();
    final n = pos.length;
    for (var i = 0; i < n; i++) {
      for (var j = 0; j < n; j++) {
        if ((i == 0 && j == 0) || (i == 0 && j == n - 1) || (i == n - 1 && j == 0)) continue;
        for (var dy = -2; dy <= 2; dy++) {
          for (var dx = -2; dx <= 2; dx++) {
            final d = dx.abs() > dy.abs() ? dx.abs() : dy.abs();
            set(pos[i] + dx, pos[j] + dy, d != 1);
          }
        }
      }
    }
    drawFormat(ecc, 0);
    if (version >= 7) {
      var rem = version;
      for (var i = 0; i < 12; i++) {
        rem = (rem << 1) ^ ((rem >> 11) * 0x1F25);
      }
      final bits = version << 12 | rem;
      for (var i = 0; i < 18; i++) {
        final bit = (bits >> i) & 1 != 0;
        final a = size - 11 + i % 3, b = i ~/ 3;
        set(a, b, bit);
        set(b, a, bit);
      }
    }
  }

  void drawFormat(QrEcc ecc, int mask) {
    final data = ecc.formatBits << 3 | mask;
    var rem = data;
    for (var i = 0; i < 10; i++) {
      rem = (rem << 1) ^ ((rem >> 9) * 0x537);
    }
    final bits = (data << 10 | rem) ^ 0x5412;
    bool bit(int i) => (bits >> i) & 1 != 0;
    for (var i = 0; i <= 5; i++) {
      set(8, i, bit(i));
    }
    set(8, 7, bit(6));
    set(8, 8, bit(7));
    set(7, 8, bit(8));
    for (var i = 9; i < 15; i++) {
      set(14 - i, 8, bit(i));
    }
    for (var i = 0; i < 8; i++) {
      set(size - 1 - i, 8, bit(i));
    }
    for (var i = 8; i < 15; i++) {
      set(8, size - 15 + i, bit(i));
    }
    set(8, size - 8, true);
  }

  void drawCodewords(List<int> data) {
    var i = 0;
    for (var right = size - 1; right >= 1; right -= 2) {
      if (right == 6) right = 5;
      for (var vert = 0; vert < size; vert++) {
        for (var j = 0; j < 2; j++) {
          final x = right - j;
          final upward = ((right + 1) & 2) == 0;
          final y = upward ? size - 1 - vert : vert;
          if (!fn[y][x] && i < data.length * 8) {
            dark[y][x] = (data[i >> 3] >> (7 - (i & 7))) & 1 != 0;
            i++;
          }
        }
      }
    }
  }

  void applyMask(int mask) {
    for (var y = 0; y < size; y++) {
      for (var x = 0; x < size; x++) {
        if (fn[y][x]) continue;
        final invert = switch (mask) {
          0 => (x + y) % 2 == 0,
          1 => y % 2 == 0,
          2 => x % 3 == 0,
          3 => (x + y) % 3 == 0,
          4 => (x ~/ 3 + y ~/ 2) % 2 == 0,
          5 => x * y % 2 + x * y % 3 == 0,
          6 => (x * y % 2 + x * y % 3) % 2 == 0,
          _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
        };
        if (invert) dark[y][x] = !dark[y][x];
      }
    }
  }

  /// The standard penalty (rules 1-4), used to choose a mask.
  int penalty() {
    var p = 0;
    bool at(int x, int y, bool rows) => rows ? dark[y][x] : dark[x][y];
    for (final rows in [true, false]) {
      for (var a = 0; a < size; a++) {
        var run = 1;
        for (var b = 1; b < size; b++) {
          if (at(b, a, rows) == at(b - 1, a, rows)) {
            run++;
            if (run == 5) p += 3;
            if (run > 5) p++;
          } else {
            run = 1;
          }
        }
        // Finder-like 1:1:3:1:1 with four light modules on one side.
        for (var b = 0; b + 7 <= size; b++) {
          const pat = [true, false, true, true, true, false, true];
          var match = true;
          for (var k = 0; k < 7 && match; k++) {
            match = at(b + k, a, rows) == pat[k];
          }
          if (!match) continue;
          bool lightRun(int from, int to) {
            for (var k = from; k < to; k++) {
              if (k >= 0 && k < size && at(k, a, rows)) return false;
            }
            return true;
          }

          if (lightRun(b - 4, b) || lightRun(b + 7, b + 11)) p += 40;
        }
      }
    }
    var darkCount = 0;
    for (var y = 0; y < size; y++) {
      for (var x = 0; x < size; x++) {
        if (dark[y][x]) darkCount++;
        if (x + 1 < size && y + 1 < size) {
          final c = dark[y][x];
          if (dark[y][x + 1] == c && dark[y + 1][x] == c && dark[y + 1][x + 1] == c) p += 3;
        }
      }
    }
    final total = size * size;
    final k = ((darkCount * 20 - total * 10).abs() + total - 1) ~/ total - 1;
    return p + (k < 0 ? 0 : k) * 10;
  }
}

/// Paints [code] with a light quiet zone of four modules, dark modules in
/// [color] on [background].
class QrView extends StatelessWidget {
  const QrView({super.key, required this.code, this.size = 184, this.color = const Color(0xFF000000), this.background = const Color(0xFFFFFFFF)});
  final QrCode code;
  final double size;
  final Color color, background;

  @override
  Widget build(BuildContext context) => SizedBox(
        width: size,
        height: size,
        child: CustomPaint(painter: _QrPainter(code, color, background)),
      );
}

class _QrPainter extends CustomPainter {
  _QrPainter(this.code, this.color, this.background);
  final QrCode code;
  final Color color, background;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRect(Offset.zero & size, Paint()..color = background);
    final n = code.size + 8;
    final m = size.width / n;
    final path = Path();
    for (var y = 0; y < code.size; y++) {
      for (var x = 0; x < code.size; x++) {
        if (code.dark(x, y)) path.addRect(Rect.fromLTWH((x + 4) * m, (y + 4) * m, m + 0.01, m + 0.01));
      }
    }
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_QrPainter old) => old.code != code || old.color != color || old.background != background;
}
