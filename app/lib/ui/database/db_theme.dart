// Database colours and sizes from design/tokens.json (`palette.*`,
// `database.*`): tag pills, board group tints, card rings and shadows, grid
// lines, the checkbox and the calendar's today circle.

import 'package:flutter/widgets.dart';

import '../ui.dart';

Color _rgba(int r, int g, int b, double a) => Color.fromRGBO(r, g, b, a);

/// Colour names in menu order.
const tagColors = [
  'default',
  'gray',
  'brown',
  'orange',
  'yellow',
  'green',
  'blue',
  'purple',
  'pink',
  'red',
];

String colorLabel(String c) =>
    c.isEmpty ? 'Default' : '${c[0].toUpperCase()}${c.substring(1)}';

class TagStyle {
  const TagStyle(this.bg, this.text);
  final Color bg, text;
}

final _tagLight = <String, TagStyle>{
  'default': TagStyle(_rgba(42, 28, 0, 0.07), const Color(0xFF2C2C2B)),
  'gray': TagStyle(_rgba(28, 19, 1, 0.11), const Color(0xFF494846)),
  'brown': TagStyle(_rgba(127, 51, 0, 0.157), const Color(0xFF584437)),
  'orange': TagStyle(_rgba(196, 88, 0, 0.204), const Color(0xFF6A4222)),
  'yellow': TagStyle(_rgba(209, 156, 0, 0.282), const Color(0xFF655121)),
  'green': TagStyle(_rgba(0, 96, 38, 0.157), const Color(0xFF2A533C)),
  'blue': TagStyle(_rgba(0, 118, 217, 0.204), const Color(0xFF264A72)),
  'purple': TagStyle(_rgba(92, 0, 163, 0.14), const Color(0xFF553B69)),
  'pink': TagStyle(_rgba(183, 0, 78, 0.153), const Color(0xFF68354E)),
  'red': TagStyle(_rgba(206, 24, 0, 0.165), const Color(0xFF6D3531)),
};

final _tagDark = <String, TagStyle>{
  'default': TagStyle(_rgba(255, 255, 243, 0.082), const Color(0xFFF0EFED)),
  'gray': TagStyle(_rgba(255, 252, 235, 0.306), const Color(0xFFF0EFED)),
  'brown': TagStyle(_rgba(255, 184, 132, 0.365), const Color(0xFFF5EDE9)),
  'orange': TagStyle(_rgba(255, 143, 71, 0.482), const Color(0xFFFBEBDE)),
  'yellow': TagStyle(_rgba(255, 188, 53, 0.46), const Color(0xFFF9F3DC)),
  'green': TagStyle(_rgba(113, 255, 175, 0.337), const Color(0xFFE8F1EC)),
  'blue': TagStyle(_rgba(81, 166, 255, 0.494), const Color(0xFFE5F2FC)),
  'purple': TagStyle(_rgba(208, 147, 255, 0.427), const Color(0xFFF3EBF9)),
  'pink': TagStyle(_rgba(255, 133, 192, 0.427), const Color(0xFFFAE9F1)),
  'red': TagStyle(_rgba(255, 116, 105, 0.525), const Color(0xFFFCE9E7)),
};

// Preserve chosen option fills while ensuring their text is readable on paper.
final _readableTagDark = <String, TagStyle>{
  for (final entry in _tagDark.entries)
    entry.key: TagStyle(
      entry.value.bg,
      _readableTagText(entry.value, AppColors.darkTheme),
    ),
};
Color _readableTagText(TagStyle style, AppColors colors) {
  final surfaces = [colors.bgPrimary, colors.desk, colors.bgSecondary];
  double contrast(Color foreground, Color background) {
    final x = foreground.computeLuminance(), y = background.computeLuminance();
    return x > y ? (x + .05) / (y + .05) : (y + .05) / (x + .05);
  }

  for (var step = 0; step <= 64; step++) {
    final candidate = Color.lerp(
      style.text,
      const Color(0xFFFFFFFF),
      step / 64,
    )!;
    if (surfaces.every(
      (surface) =>
          contrast(candidate, Color.alphaBlend(style.bg, surface)) >= 4.5,
    )) {
      return candidate;
    }
  }
  return const Color(0xFFFFFFFF);
}

/// `palette.text`: the colour's text tone (same in light and dark).
const _textTone = <String, Color>{
  'gray': Color(0xFF7D7A75),
  'brown': Color(0xFF9F765A),
  'orange': Color(0xFFD27B2D),
  'yellow': Color(0xFFCB9434),
  'green': Color(0xFF50946E),
  'blue': Color(0xFF387DC9),
  'purple': Color(0xFF9A6BB4),
  'pink': Color(0xFFC14C8A),
  'red': Color(0xFFCF5148),
};

final _groupLight = <String, Color>{
  'gray': _rgba(66, 35, 3, 0.03),
  'brown': _rgba(115, 59, 3, 0.035),
  'orange': _rgba(186, 72, 3, 0.043),
  'yellow': _rgba(207, 175, 0, 0.063),
  'green': _rgba(3, 87, 31, 0.035),
  'blue': _rgba(0, 128, 213, 0.047),
  'purple': _rgba(98, 3, 161, 0.03),
  'pink': _rgba(161, 3, 66, 0.03),
  'red': _rgba(199, 3, 3, 0.035),
};

final _groupDark = <String, Color>{
  'gray': _rgba(252, 252, 252, 0.03),
  'brown': _rgba(253, 158, 101, 0.05),
  'orange': _rgba(254, 139, 71, 0.043),
  'yellow': _rgba(255, 232, 48, 0.043),
  'green': _rgba(83, 255, 140, 0.035),
  'blue': _rgba(41, 139, 253, 0.063),
  'purple': _rgba(196, 101, 253, 0.05),
  'pink': _rgba(255, 78, 149, 0.055),
  'red': _rgba(251, 107, 107, 0.047),
};

/// `palette.background` (card rings of coloured board groups).
final _bgLight = <String, Color>{
  'gray': _rgba(42, 28, 0, 0.07),
  'brown': _rgba(139, 46, 0, 0.086),
  'orange': _rgba(224, 101, 1, 0.13),
  'yellow': _rgba(211, 168, 0, 0.137),
  'green': _rgba(0, 100, 45, 0.09),
  'blue': _rgba(0, 124, 215, 0.094),
  'purple': _rgba(102, 0, 178, 0.08),
  'pink': _rgba(197, 0, 93, 0.086),
  'red': _rgba(223, 22, 0, 0.094),
};

final _bgDark = <String, Color>{
  'gray': _rgba(255, 255, 235, 0.1),
  'brown': _rgba(255, 177, 129, 0.192),
  'orange': _rgba(255, 147, 75, 0.28),
  'yellow': _rgba(255, 205, 75, 0.24),
  'green': _rgba(108, 255, 172, 0.157),
  'blue': _rgba(67, 155, 255, 0.24),
  'purple': _rgba(200, 125, 255, 0.2),
  'pink': _rgba(255, 103, 177, 0.23),
  'red': _rgba(255, 104, 92, 0.24),
};

/// `palette.callout.dark`: dark cards of coloured board groups.
const _cardDark = <String, Color>{
  'gray': Color(0xFF2A2A29),
  'brown': Color(0xFF33291F),
  'orange': Color(0xFF3D2A1D),
  'yellow': Color(0xFF3A3220),
  'green': Color(0xFF1F2F26),
  'blue': Color(0xFF213041),
  'purple': Color(0xFF2E2536),
  'pink': Color(0xFF38222D),
  'red': Color(0xFF3A2321),
};

extension DbColors on AppColors {
  TagStyle tag(String? color) {
    if (color == null || color == 'default') {
      return TagStyle(bgSecondary, textPrimary);
    }
    final m = dark ? _readableTagDark : _tagLight;
    return m[color] ?? TagStyle(bgSecondary, textPrimary);
  }

  /// The dot of a status pill and the count of a board group.
  Color tone(String? color) => _textTone[color] ?? textSecondary;

  Color groupTint(String? color) =>
      (dark ? _groupDark : _groupLight)[color] ??
      (dark ? _groupDark : _groupLight)['gray']!;

  Color cardRing(String? color) =>
      (dark ? _bgDark : _bgLight)[color] ??
      (dark ? _rgba(255, 255, 243, 0.082) : _rgba(42, 28, 0, 0.07));

  Color cardBg(String? color) =>
      dark ? (_cardDark[color] ?? bgPrimary) : bgPrimary;

  /// Table grid lines (`database.table.gridLine*`).
  Color get gridLine =>
      dark ? _rgba(255, 255, 243, 0.082) : _rgba(42, 28, 0, 0.07);

  /// Calendar and timeline lines.
  Color get calendarLine => border;

  Color get checkboxFill => accent;
  Color get checkboxBorder => threadStrong;

  Color get todayRed => const Color(0xFFE56458);

  /// Board and gallery card shadows plus a 1 px ring of [ring].
  List<BoxShadow> cardShadow(Color ring, {bool lifted = false}) => [
    if (lifted) ...[
      BoxShadow(
        color: _rgba(25, 25, 25, dark ? 0.3 : 0.12),
        offset: const Offset(0, 8),
        blurRadius: 24,
      ),
    ] else if (dark)
      BoxShadow(
        color: _rgba(25, 25, 25, 0.08),
        offset: const Offset(0, 2),
        blurRadius: 4,
      )
    else ...[
      BoxShadow(
        color: _rgba(25, 25, 25, 0.027),
        offset: const Offset(0, 4),
        blurRadius: 12,
      ),
      BoxShadow(
        color: _rgba(25, 25, 25, 0.02),
        offset: const Offset(0, 1),
        blurRadius: 2,
      ),
    ],
    BoxShadow(color: ring, spreadRadius: 1),
  ];
}

/// Database sizes (desktop unless noted).
abstract final class Db {
  static const gutter = 96.0;
  static const innerPad = 8.0;
  static const rowHeight = 36.0;
  static const headerHeight = 36.0;
  static const defaultColumn = 200.0;
  static const titleColumn = 280.0;
  static const minColumn = 90.0;
  static const boardColumn = 276.0;
  static const boardGap = 12.0;
  static const cardWidth = 260.0;
  static const mobileRowHeight = 47.0;
}

/// Calendar and timeline entries inherit their database's space colour.
Color eventPillFill(AppColors colors, String? name) =>
    spaceColor(
      name,
      colors.dark ? Brightness.dark : Brightness.light,
    )?.withValues(alpha: .25) ??
    colors.infoBg;
