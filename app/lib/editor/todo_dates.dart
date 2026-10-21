// A to-do date is still the editor's ordinary ISO Date mark. Only its
// resting presentation changes; the stored text and UTF-16 ranges stay intact.
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../addons/database/api.dart';
import '../ui/database/values.dart';
import '../ui/database/widgets.dart';
import '../registry/property_registry.dart';
import '../ui/ui.dart';
import 'mention_menu.dart';
import 'model.dart';

DateTime? todoDate(String iso) {
  if (!RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(iso)) return null;
  final day = DateTime.tryParse(iso);
  return day != null && isoDate(day) == iso ? day : null;
}

int todoDateDistance(DateTime day) {
  final now = clockNow();
  return DateTime.utc(day.year, day.month, day.day)
      .difference(DateTime.utc(now.year, now.month, now.day)).inDays;
}

String todoDateLabel(DateTime day) {
  final distance = todoDateDistance(day);
  if (distance == 0) return 'Today';
  if (distance == 1) return 'Tomorrow';
  if (distance == -1) return 'Yesterday';
  if (distance.abs() <= 6) return const ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'][day.weekday - 1];
  const months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  return '${day.day} ${months[day.month - 1]}${day.year == clockNow().year ? '' : ' ${day.year}'}';
}

BMark? firstTodoDate(Node node) {
  final dates = node.marks.where((m) => m.kind == BMarkKind.date && m.start >= 0 && m.end <= node.text.length && m.start < m.end).toList()
    ..sort((a, b) => a.start.compareTo(b.start));
  return dates.firstOrNull;
}

typedef TodoDateEdit = ({String text, List<BMark> marks});

/// Replace the first date, keeping surrounding text and formatting, or
/// append the same visible text and mark that the mention menu creates.
TodoDateEdit replaceTodoDate(Node node, DateTime? day) {
  final first = firstTodoDate(node);
  if (first == null && day == null) return (text: node.text, marks: List.of(node.marks));
  var start = first?.start ?? node.text.length;
  final end = first?.end ?? start;
  var replacement = day == null ? '' : '@${longDate(day)}';
  if (first == null && node.text.isNotEmpty) replacement = ' $replacement';
  if (day == null && start > 0 && node.text[start - 1] == ' ') start--;
  final delta = replacement.length - (end - start);
  final text = node.text.replaceRange(start, end, replacement);
  final marks = <BMark>[];
  for (final m in node.marks) {
    if (identical(m, first)) continue;
    var a = m.start, b = m.end;
    if (a >= end) { a += delta; b += delta; }
    else if (b > start) {
      a = a <= start ? a : start;
      b = b >= end ? b + delta : start + replacement.length;
    }
    if (a < b) marks.add(BMark(start: a, end: b, kind: m.kind, url: m.url));
  }
  if (day != null) {
    final dateStart = start + (first == null && node.text.isNotEmpty ? 1 : 0);
    marks.add(BMark(start: dateStart, end: start + replacement.length, kind: BMarkKind.date, url: isoDate(day)));
  }
  marks.sort((a, b) => a.start.compareTo(b.start));
  return (text: text, marks: marks);
}

Future<void> editTodoDate(BuildContext context, Rect anchor, Node node, ValueChanged<TodoDateEdit> onChanged) {
  final mark = firstTodoDate(node);
  final day = mark == null ? null : todoDate(mark.url);
  final def = const PropDef(key: '', name: 'Date', kind: PropKind.date, options: [], config: '');
  final args = EditArgs(
    context: context, anchor: anchor, def: def, collection: '', saveDef: (d) async => d,
    value: day == null ? Values.empty : Values.date(DateTime.utc(day.year, day.month, day.day).millisecondsSinceEpoch),
    onChanged: (value) {
      final d = value.kind == ValueKind.date ? DateTime.fromMillisecondsSinceEpoch(value.startMs, isUtc: true) : null;
      onChanged(replaceTodoDate(node, d));
    },
  );
  return showPanel<void>(context, anchor,
    (_) => DatePicker(args: args, dayOnly: true, clearLabel: 'Remove date'), width: 280);
}
