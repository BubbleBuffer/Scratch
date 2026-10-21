// The date editor: the chosen date (and end date), a month grid, "End
// date" and "Include time" switches, time fields, and "Clear". Whole-day
// dates are saved as midnight UTC; dates with a time as instants.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../registry/property_registry.dart';
import '../../../ui/ui.dart';
import '../../../ui/database/db_theme.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';

Future<void> editDate(EditArgs a) => showPanel<void>(a.context, a.anchor, (ctx) => DatePicker(args: a), width: 280);

class DatePicker extends StatefulWidget {
  const DatePicker({super.key, required this.args, this.dayOnly = false, this.clearLabel = 'Clear'});
  final EditArgs args;
  /// To-do dates are calendar days: choose once, with no range or time.
  final bool dayOnly;
  final String clearLabel;

  @override
  State<DatePicker> createState() => _DatePickerState();
}

class _DatePickerState extends State<DatePicker> {
  Day? start, end;
  bool hasTime = false;
  int startMin = 9 * 60, endMin = 10 * 60;
  bool editingEnd = false;
  late Day month;

  @override
  void initState() {
    super.initState();
    final v = widget.args.value;
    if (v.kind == ValueKind.date) {
      hasTime = v.hasTime;
      start = Day.ofMs(v.startMs, v.hasTime);
      if (v.hasTime) {
        final t = DateTime.fromMillisecondsSinceEpoch(v.startMs);
        startMin = t.hour * 60 + t.minute;
      }
      final e = v.endMs;
      if (e != null) {
        end = Day.ofMs(e, v.hasTime);
        if (v.hasTime) {
          final t = DateTime.fromMillisecondsSinceEpoch(e);
          endMin = t.hour * 60 + t.minute;
        }
      }
    }
    final base = start ?? Day.of(clockNow());
    month = Day(base.year, base.month, 1);
  }

  int _ms(Day d, int minutes) =>
      hasTime ? DateTime(d.year, d.month, d.day, minutes ~/ 60, minutes % 60).millisecondsSinceEpoch : d.ms;

  void _emit() {
    final s = start;
    if (s == null) {
      widget.args.onChanged(Values.empty);
      return;
    }
    var e = end;
    if (e != null && e.compareTo(s) < 0) e = s;
    widget.args.onChanged(Values.date(_ms(s, startMin), end: e == null ? null : _ms(e, endMin), hasTime: hasTime));
  }

  void _pickDay(Day d) {
    setState(() {
      if (end != null && editingEnd) {
        end = d;
        if (start != null && d.compareTo(start!) < 0) {
          end = start;
          start = d;
        }
        editingEnd = false;
      } else {
        start = d;
        if (end != null) {
          if (end!.compareTo(d) < 0) end = d;
          editingEnd = true;
        }
      }
    });
    _emit();
    if (widget.dayOnly) Navigator.of(context).maybePop();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final today = Day.of(clockNow());
    final first = month;
    final lead = first.weekday0Sunday();
    final gridStart = first.add(-lead);
    final m = context.mobile;
    final cell = m ? 40.0 : 32.0;

    Widget field(Day? d, int minutes, bool isEnd) {
      final active = end != null && editingEnd == isEnd;
      return Expanded(
        child: GestureDetector(
          onTap: () => setState(() => editingEnd = isEnd),
          child: Container(
            height: m ? 36 : 28,
            padding: const EdgeInsets.symmetric(horizontal: 6),
            decoration: BoxDecoration(
              color: c.bgSecondary,
              borderRadius: Radii.field,
              border: Border.all(color: active ? c.accent : c.borderTranslucent),
            ),
            alignment: Alignment.centerLeft,
            child: Text(
              d == null ? '' : '${formatDay(d)}${hasTime ? ' ${_fmtMin(minutes)}' : ''}',
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(ty.ui, c.textPrimary, weight: 400),
            ),
          ),
        ),
      );
    }

    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Row(children: [
        field(start, startMin, false),
        if (end != null) ...[const SizedBox(width: 6), field(end, endMin, true)],
      ]),
      const SizedBox(height: 8),
      Row(children: [
        const SizedBox(width: 6),
        Expanded(
            child: Text('${monthName(month.month)} ${month.year}',
                key: const ValueKey('date-month'), style: ts(ty.ui, c.textPrimary, weight: 600))),
        TextBtn('Today', height: 24, color: c.textSecondary, onTap: () => setState(() => month = Day(today.year, today.month, 1))),
        IconBtn(Ico.chevronLeft,
            key: const ValueKey('date-prev'),
            size: 24,
            glyph: 16,
            onTap: () => setState(() => month = _addMonths(month, -1)),
            semanticLabel: 'Previous month'),
        IconBtn(Ico.chevronRight,
            key: const ValueKey('date-next'),
            size: 24,
            glyph: 16,
            onTap: () => setState(() => month = _addMonths(month, 1)),
            semanticLabel: 'Next month'),
      ]),
      const SizedBox(height: 4),
      Row(children: [
        for (final w in ['Su', 'Mo', 'Tu', 'We', 'Th', 'Fr', 'Sa'])
          Expanded(child: Center(child: Text(w, style: ts(ty.uiSmall, c.textTertiary, weight: 400)))),
      ]),
      for (var week = 0; week < 6; week++)
        Row(children: [
          for (var i = 0; i < 7; i++)
            Expanded(child: _dayCell(context, gridStart.add(week * 7 + i), today, cell)),
        ]),
      const SizedBox(height: 4),
      const MenuSeparator(),
      if (!widget.dayOnly) MenuItem(
        key: const ValueKey('date-end-toggle'),
        label: 'End date',
        onTap: () {
          setState(() {
            end = end == null ? (start ?? today) : null;
            editingEnd = end != null;
          });
          _emit();
        },
        trailing: Toggle(on: end != null),
      ),
      if (!widget.dayOnly) MenuItem(
        key: const ValueKey('date-time-toggle'),
        label: 'Include time',
        onTap: () {
          setState(() => hasTime = !hasTime);
          _emit();
        },
        trailing: Toggle(on: hasTime),
      ),
      if (hasTime)
        Padding(
          padding: const EdgeInsets.fromLTRB(8, 2, 8, 4),
          child: Row(children: [
            Expanded(
              child: MenuField(
                key: const ValueKey('date-start-time'),
                initial: _fmtMin(startMin),
                onSubmitted: (s) {
                  final v = _parseMin(s);
                  if (v != null) setState(() => startMin = v);
                  _emit();
                },
              ),
            ),
            if (end != null) ...[
              const SizedBox(width: 6),
              Expanded(
                child: MenuField(
                  key: const ValueKey('date-end-time'),
                  initial: _fmtMin(endMin),
                  onSubmitted: (s) {
                    final v = _parseMin(s);
                    if (v != null) setState(() => endMin = v);
                    _emit();
                  },
                ),
              ),
            ],
          ]),
        ),
      const MenuSeparator(),
      MenuItem(
        key: const ValueKey('date-clear'),
        label: widget.clearLabel,
        onTap: () {
          setState(() {
            start = null;
            end = null;
          });
          _emit();
          Navigator.of(context).maybePop();
        },
      ),
    ]);
  }

  Widget _dayCell(BuildContext context, Day d, Day today, double size) {
    final c = context.colors;
    final inMonth = d.month == month.month;
    final isStart = d == start;
    final isEnd = d == end;
    final s = start, e = end;
    final inRange = s != null && e != null && d.compareTo(s) > 0 && d.compareTo(e) < 0;
    final picked = isStart || isEnd;
    return GestureDetector(
      key: ValueKey('day-${d.year}-${d.month}-${d.day}'),
      behavior: HitTestBehavior.opaque,
      onTap: () => _pickDay(d),
      child: Container(
        height: size,
        color: inRange ? c.accent.withValues(alpha: 0.12) : null,
        alignment: Alignment.center,
        child: Hover(
          onTap: () => _pickDay(d),
          radius: size / 2,
          child: Container(
            width: size - 4,
            height: size - 4,
            alignment: Alignment.center,
            decoration: picked ? BoxDecoration(color: c.accent, shape: BoxShape.circle) : null,
            child: Text('${d.day}',
                style: ts(
                  context.type.ui,
                  picked
                      ? c.onAccent
                      : d == today
                          ? c.todayRed
                          : inMonth
                              ? c.textPrimary
                              : c.textTertiary,
                  weight: d == today || picked ? 600 : 400,
                )),
          ),
        ),
      ),
    );
  }
}

Day _addMonths(Day d, int n) {
  final m = d.month - 1 + n;
  return Day(d.year + (m / 12).floor(), m % 12 + 1, 1);
}

String _fmtMin(int m) => formatTime(DateTime(2000, 1, 1, m ~/ 60, m % 60));

/// "9:30 AM", "21:15", "9pm" -> minutes after midnight.
int? _parseMin(String s) {
  final t = s.trim().toLowerCase();
  final r = RegExp(r'^(\d{1,2})(?::(\d{2}))?\s*(am|pm|a|p)?$').firstMatch(t);
  if (r == null) return null;
  var h = int.parse(r.group(1)!);
  final min = int.tryParse(r.group(2) ?? '0') ?? 0;
  final ap = r.group(3);
  if (ap != null) {
    if (h == 12) h = 0;
    if (ap.startsWith('p')) h += 12;
  }
  if (h > 23 || min > 59) return null;
  return h * 60 + min;
}
