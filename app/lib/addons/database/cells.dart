// Value displays shared by the property types: plain text, pills, the
// "Empty" placeholder of page rows, and the text style of each place a
// value is drawn.

import 'package:flutter/widgets.dart';

import '../../core/models.dart' hide StatusGroup;
import '../../registry/property_registry.dart';
import '../../ui/ui.dart';
import '../../ui/database/config.dart';
import '../../ui/database/db_theme.dart';
import '../../ui/database/values.dart';
import '../../ui/database/widgets.dart';

/// The value text style of [mode].
TextStyle cellStyle(BuildContext context, CellMode mode, {Color? color, int weight = 400}) {
  final c = context.colors;
  final m = context.mobile;
  final spec = switch (mode) {
    CellMode.table => m ? const TypeSpec(16, 20, 400) : const TypeSpec(14, 21, 400),
    CellMode.card => const TypeSpec(12, 18, 400),
    CellMode.list => const TypeSpec(14, 21, 400),
    CellMode.page => m ? const TypeSpec(15, 20, 400) : const TypeSpec(14, 20, 400),
  };
  return ts(spec, color ?? c.textPrimary, weight: weight);
}

/// "Empty" in page rows; nothing elsewhere.
Widget emptyCell(CellArgs a) => a.mode == CellMode.page
    ? Text('Empty', style: cellStyle(a.context, a.mode, color: a.context.colors.textTertiary))
    : const SizedBox.shrink();

/// Text, one line with an ellipsis unless wrapping.
Widget textCell(CellArgs a, String s, {bool underline = false, TextAlign align = TextAlign.start}) {
  if (s.isEmpty) return emptyCell(a);
  final c = a.context.colors;
  var style = cellStyle(a.context, a.mode);
  if (underline) style = style.copyWith(color: c.accentText, decoration: TextDecoration.underline, decorationColor: c.accentText);
  final wrap = a.wrap || a.mode == CellMode.page;
  return Text(s,
      maxLines: wrap ? null : 1,
      softWrap: wrap,
      overflow: wrap ? null : TextOverflow.ellipsis,
      textAlign: align,
      style: style);
}

/// Option pills of a select, multi-select or status value.
Widget pillsCell(CellArgs a) {
  final ids = a.value.optionIds;
  if (ids.isEmpty) return emptyCell(a);
  final status = a.def.kind == PropKind.status;
  final small = a.mode == CellMode.card || a.mode == CellMode.list;
  final pills = [
    for (final id in ids)
      if (a.def.option(id) case final o?) Pill(o.name, color: o.color, status: status,
          group: status ? statusGroupOf(a.def, o.id) : null, small: small) else Pill(id, small: small),
  ];
  if (a.wrap || a.mode == CellMode.page || a.mode == CellMode.card) {
    return Wrap(spacing: a.mode == CellMode.card ? 6 : 8, runSpacing: 6, children: pills);
  }
  return ClipRect(
    child: OverflowBox(
      alignment: Alignment.centerLeft,
      maxWidth: double.infinity,
      child: Row(mainAxisSize: MainAxisSize.min, children: [
        for (var i = 0; i < pills.length; i++) ...[if (i > 0) const SizedBox(width: 6), pills[i]],
      ]),
    ),
  );
}

/// The date value text in the property's format.
String dateText(PropValue v, PropDef def) {
  if (v.kind != ValueKind.date) return '';
  final f = PropConfig(def.config).dateFormat;
  if (f == 'relative') {
    final d = Day.ofMs(v.startMs, v.hasTime);
    final today = Day.of(clockNow());
    final diff = d - today;
    final day = switch (diff) {
      0 => 'Today',
      1 => 'Tomorrow',
      -1 => 'Yesterday',
      _ => formatDay(d),
    };
    return v.hasTime ? '$day ${formatTime(DateTime.fromMillisecondsSinceEpoch(v.startMs))}' : day;
  }
  return formatDate(v, format: f);
}

/// Colour used for "Empty" and hints (kept here so cells need one import).
Color hintColor(BuildContext context) => context.colors.textTertiary;

/// The checkbox cell.
Widget checkboxCell(CellArgs a) {
  final size = a.mode == CellMode.card ? 14.0 : (a.context.mobile ? 18.0 : 16.0);
  return DbCheckbox(checked: a.value.kind == ValueKind.checkbox && a.value.checked, size: size);
}

/// A row of the card or list: value only, no label.
bool shownOnCard(PropValue v) => !v.isEmpty && !(v.kind == ValueKind.checkbox && !v.checked);

/// Tag colours are in db_theme; re-exported for property files.
TagStyle tagStyle(BuildContext context, String? color) => context.colors.tag(color);
