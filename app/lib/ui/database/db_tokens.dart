// Database colours and sizes from design/tokens.json (`palette.*`,
// `database.*`): tag pills, board group tints, card rings and shadows, grid
// lines, the checkbox and the calendar's today circle. A token library: the
// database views read their particular values from here by name. Text tones
// and card rings are the block palette's (block_color_tokens.dart).

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
  Color tone(String? color) => BlockPalette.text[color] ?? textSecondary;

  Color groupTint(String? color) =>
      (dark ? _groupDark : _groupLight)[color] ??
      (dark ? _groupDark : _groupLight)['gray']!;

  Color cardRing(String? color) =>
      (dark ? BlockPalette.backgroundDark : BlockPalette.backgroundLight)[color] ??
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
  static const titleColumnCompact = 160.0, defaultColumnCompact = 140.0;
  static const boardColumn = 276.0;
  static const boardGap = 12.0;
  static const cardWidth = 260.0;
  static const mobileRowHeight = 47.0;

  /// A table: its header by touch, the "+" column for a new property (with
  /// a pointer), the new-row row with a pointer and by touch, the totals
  /// row, and a column's resize grip.
  static const headerHeightTouch = 40.0, addColumn = 34.0;
  static const addRow = 34.0, addRowTouch = 45.0, calcRow = 36.0;
  static const resizeGrip = 7.0, resizeGripReach = 3.0;

  /// An option pill's height, and a board or gallery card's smaller pill.
  static const pillHeight = 22.0, smallPillHeight = 18.0;

  /// A status pill's dot on a card's smaller pill.
  static const smallPillDot = 6.0;

  /// The checkbox of checkbox properties.
  static const checkbox = 16.0;

  /// The longest value beside a menu row's label.
  static const menuValue = 140.0;

  /// A toolbar or rule chip's corners, and its longest label.
  static const chipRadius = 14.0, chipLabel = 220.0;

  /// The room under a view's last row.
  static const endPadding = 120.0;

  /// The narrowest editor over a cell.
  static const cellEditorMin = 240.0;

  /// A full-page database's side margin, on a wide screen and narrower.
  static const pageMarginWide = 96.0, pageMargin = 48.0, wideAt = 900.0;

  /// Room kept for a view while it loads.
  static const loading = 120.0;

  /// The views tab bar, a tab's corners and the longest tab name.
  static const tabBar = 40.0, tabRadius = 16.0, tabLabel = 220.0;

  /// The toolbar's search field and its "New" button.
  static const searchField = 150.0, newButton = 72.0;

  /// The rules bar under the tabs (sorts and filters).
  static const rulesBar = 30.0;

  /// The view switcher on a compact screen, its pill's corners, and the
  /// views sheet's header and rows.
  static const switcherRadius = 18.0, sheetHeader = 52.0;

  /// The checkbox in a card (smaller) and in a cell by touch (larger).
  static const cardCheckbox = 14.0, touchCheckbox = 18.0;

  /// The date picker's day cell, with a pointer and by touch, and the inset
  /// of the picked day's circle.
  static const dayCell = 32.0, dayCellTouch = 40.0, dayInset = 4.0;

  /// The option editor: the chip row's least height, a new option's field,
  /// and a colour swatch.
  static const optionRow = 34.0, optionFieldMin = 80.0, optionFieldMax = 260.0;
  static const swatch = 18.0;

  /// The relation picker's list.
  static const relationList = 280.0;

  /// The text cell editor: one line at least, and the most it grows.
  static const cellEditorLine = 21.0, cellEditorMax = 300.0;

  /// The filter editor's "where/and/or" column.
  static const filterLead = 64.0;

  /// A page's property row (at least), with a pointer and by touch, and its
  /// label column.
  static const propRow = 30.0, propRowTouch = 34.0, propLabel = 140.0;

  /// A layout tile in the view settings.
  static const layoutTile = 62.0;

  /// The longest list in a settings menu before it scrolls.
  static const settingsList = 320.0;
}

/// The "Open" chip on a row a pointer hovers, with its little peek glyph,
/// with a pointer and by touch.
abstract final class DbOpenChip {
  static const height = 22.0, heightTouch = 26.0;
  static const glyph = Size(12, 10), glyphPane = 4.0;
}

/// The gallery: a card's least width by card size (compact screens take
/// narrower small cards and wider medium ones), the cover's height, and the
/// room a card keeps while its row loads.
abstract final class DbGallery {
  static const small = 180.0, smallCompact = 150.0;
  static const medium = 260.0, mediumCompact = 300.0;
  static const large = 340.0, largeCompact = 320.0;
  static const smallCover = 110.0, mediumCover = 150.0, largeCover = 220.0;
  static const loadingCard = 60.0;
}

/// The list view's row, with a pointer and by touch, and a value's widest.
abstract final class DbList {
  static const row = 34.0, rowTouch = 44.0, value = 280.0;

  /// How far rows reach into the gutter.
  static const reach = 6.0;
}

/// The timeline view: a lane, an entry's bar in it and the bar's resize
/// handles, the room a narrow bar's label takes beside it, the date header
/// and the room under the last lane.
abstract final class DbTimeline {
  static const row = 36.0, bar = 26.0, handle = 8.0, narrowLabel = 400.0;
  static const header = 48.0, endPadding = 80.0;

  /// A bar's inset from its days, and the least share of a day it takes.
  static const barInset = 4.0, minBarShare = .5;

  /// The lanes an inline timeline keeps at least.
  static const inlineLanes = 4;
}

/// The calendar view.
abstract final class DbCalendar {
  /// A week of the month grid; an entry's bar, the gap between bars, the
  /// first bar's top (under the day's number) and a bar's inset from its
  /// days' edges.
  static const row = 124.0, bar = 22.0, barGap = 3.0, barTop = 30.0;
  static const barInset = 4.0;

  /// The weekday names over the grid.
  static const weekdays = 26.0;

  /// The compact month: the weekday names, a week, the day's circle and its
  /// dots.
  static const compactWeekdays = 30.0, compactWeek = 60.0, compactDay = 30.0;
  static const dotRow = 5.0, dot = 4.0;

  /// A day's number (today's in a pill), and a dragged entry's widest.
  static const dayLabel = 24.0, dragged = 220.0;

  /// The room under the last week.
  static const endPadding = 80.0;
}

/// Calendar and timeline entries inherit their database's space colour.
Color eventPillFill(AppColors colors, String? name) =>
    spaceColor(
      name,
      colors.dark ? Brightness.dark : Brightness.light,
    )?.withValues(alpha: Alpha.wash) ??
    colors.infoBg;
