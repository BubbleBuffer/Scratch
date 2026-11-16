// The calendar view: a month grid of the rows by a date property. Entries
// spanning days are bars across the week; drag one to another day to
// reschedule it; "+" on a day adds a row on that date. Only the shown
// weeks are queried. On a compact screen, a compact month with dots and the chosen
// day's rows below it.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../../../../core/models.dart';
import '../../../../ui/database/config.dart';
import '../../../../ui/database/db_icons.dart';
import '../../../../ui/database/db_tokens.dart';
import '../../property_menu.dart';
import '../../../../ui/database/values.dart';
import '../../../../ui/database/widgets.dart';
import '../../../../registry/view_registry.dart';
import '../../../../ui/ui.dart';
import '../common.dart';
import '../dates.dart';

final calendarView = ViewType(
  kind: ViewKind.calendar,
  label: 'Calendar',
  icon: DbIco.calendar,
  needsDate: true,
  build: (context, a) => CalendarView(args: a),
  options: (context, a) => [
    ViewOption(
      label: 'Show calendar by',
      value: menuValue(
        context,
        a.store.def(a.view.dateProp ?? '')?.name ?? 'None',
      ),
      onTap: (ctx, rect) => pickDateProp(ctx, rect, a),
    ),
    ViewOption(
      label: 'Start week on Monday',
      value: Toggle(on: a.view.cfg.mondayStart),
      onTap: (_, _) =>
          a.store.updateConfig(a.view, (c) => c.mondayStart = !c.mondayStart),
    ),
  ],
);

/// Picks the date property a calendar or timeline shows.
Future<void> pickDateProp(BuildContext ctx, Rect rect, ViewArgs a) async {
  final dates = [
    for (final d in a.store.schema)
      if (d.kind == PropKind.date ||
          d.kind == PropKind.createdTime ||
          d.kind == PropKind.updatedTime)
        d,
  ];
  final k = await showPanel<String>(
    ctx,
    rect,
    (c) => Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final d in dates)
          dbItem(
            c,
            d.name,
            propIcon(d),
            key: ValueKey('date-prop-${d.name}'),
            trailing: a.view.dateProp == d.key
                ? Icon20(Ico.check, size: IconSize.sm, color: c.colors.iconPrimary)
                : null,
            onTap: () => Navigator.of(c).pop(d.key),
          ),
      ],
    ),
    width: 220,
  );
  if (k != null) await a.store.updateView(a.view.copyWith(dateProp: k));
}

class CalendarView extends StatefulWidget {
  const CalendarView({super.key, required this.args});
  final ViewArgs args;

  @override
  State<CalendarView> createState() => _CalendarViewState();
}

class _CalendarViewState extends State<CalendarView> {
  late Day _month = _firstOfMonth(Day.of(clockNow()));
  Day? _selected;
  int _grab = 0;

  ViewArgs get a => widget.args;

  static Day _firstOfMonth(Day d) => Day(d.year, d.month, 1);

  int get _weekStart => a.view.cfg.mondayStart ? 1 : 0;

  /// The first day of the grid (the week containing the 1st).
  Day get _gridStart {
    final back = (_month.weekday0Sunday() - _weekStart + 7) % 7;
    return _month.add(-back);
  }

  int get _weeks {
    final next = Day(
      _month.month == 12 ? _month.year + 1 : _month.year,
      _month.month == 12 ? 1 : _month.month + 1,
      1,
    );
    return ((next - _gridStart) / 7).ceil();
  }

  void _shiftMonth(int d) {
    setState(() {
      final m = _month.month - 1 + d;
      _month = Day(_month.year + (m / 12).floor(), m % 12 + 1, 1);
    });
  }

  Future<void> _addOn(Day day) async {
    final key = a.view.dateProp;
    if (key == null) return;
    await a.ctl.newRow(context, values: {key: Values.date(day.ms)});
  }

  Future<void> _moveTo(RangeEntry e, Day target) async {
    final key = a.view.dateProp;
    if (key == null) return;
    final delta = target.add(-_grab) - entryStart(e);
    _grab = 0;
    if (delta == 0) return;
    await a.store.setValue(e.page.id, key, shiftedValue(e, delta, delta));
  }

  List<RangeEntry> _entries(Day first, Day last) {
    final key = a.view.dateProp;
    return [
      for (final e in a.store.rangeEntries)
        if (e.prop == key &&
            entryStart(e).compareTo(last) <= 0 &&
            entryEnd(e).compareTo(first) >= 0)
          e,
    ];
  }

  @override
  Widget build(BuildContext context) {
    final key = a.view.dateProp;
    final c = context.colors;
    if (key == null || a.store.def(key) == null) {
      return Padding(
        padding: EdgeInsets.fromLTRB(a.gutter, Space.xl, a.gutter, Space.xl),
        child: Builder(
          builder: (b) => ChipBtn(
            key: const ValueKey('calendar-pick-date'),
            label: 'Choose a date property',
            icon: DbIco.date,
            onTap: () => pickDateProp(b, globalRectOf(b), a),
          ),
        ),
      );
    }
    final first = _gridStart;
    final last = first.add(_weeks * 7 - 1);
    final (from, to) = queryRangeOf(first, last);
    a.store.showRange(key, from, to);
    // A compact screen gets the compact month with the chosen day's list.
    final m = context.sizeClass.isCompact;
    final head = Padding(
      padding: EdgeInsets.fromLTRB(a.gutter + Space.sm, Space.xxs, a.gutter, Space.xxs),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: (context.touch ? ControlSize.xl : ControlSize.md).height),
        child: Row(
          children: [
            Text(
              '${monthName(_month.month)} ${_month.year}',
              key: const ValueKey('calendar-month'),
              style: ts(
                m ? context.type.title : context.type.ui,
                c.textPrimary,
                weight: Weight.semibold,
              ),
            ),
            const Spacer(),
            IconBtn(
              Ico.chevronLeft,
              key: const ValueKey('calendar-prev'),
              size: context.touch ? ControlSize.lg : ControlSize.xs,
              glyph: context.touch ? IconSize.lg : IconSize.sm,
              color: c.iconSecondary,
              semanticLabel: 'Previous month',
              onTap: () => _shiftMonth(-1),
            ),
            TextBtn(
              'Today',
              key: const ValueKey('calendar-today'),
              size: m ? ControlSize.lg : ControlSize.xs,
              color: c.textSecondary,
              onTap: () {
                setState(() {
                  _month = _firstOfMonth(Day.of(clockNow()));
                  _selected = Day.of(clockNow());
                });
              },
            ),
            IconBtn(
              Ico.chevronRight,
              key: const ValueKey('calendar-next'),
              size: context.touch ? ControlSize.lg : ControlSize.xs,
              glyph: context.touch ? IconSize.lg : IconSize.sm,
              color: c.iconSecondary,
              semanticLabel: 'Next month',
              onTap: () => _shiftMonth(1),
            ),
          ],
        ),
      ),
    );
    final entries = _entries(first, last);
    final body = m
        ? _mobile(context, first, entries)
        : _desktop(context, first, entries);
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: a.inline ? MainAxisSize.min : MainAxisSize.max,
      children: [
        head,
        if (a.inline) body else Expanded(child: body),
      ],
    );
  }

  // -------------------------------------------------------------- desktop

  Widget _desktop(BuildContext context, Day first, List<RangeEntry> entries) {
    final c = context.colors;
    final names = [
      for (var i = 0; i < 7; i++) weekdaysShort[(i + _weekStart) % 7],
    ];
    final grid = LayoutBuilder(
      builder: (context, box) {
        final w = box.maxWidth - 2 * a.gutter + (a.inline ? a.gutter : 0);
        final cw = (w - Stroke.hair) / DateTime.daysPerWeek; // the grid's left border
        return Padding(
          padding: EdgeInsets.only(
            left: a.gutter,
            right: a.inline ? 0 : a.gutter,
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            mainAxisSize: MainAxisSize.min,
            children: [
              SizedBox(
                height: DbCalendar.weekdays,
                child: Row(
                  children: [
                    for (final n in names)
                      SizedBox(
                        width: cw,
                        child: Center(
                          child: Text(
                            n,
                            style: ts(
                              context.type.uiSmall,
                              c.textTertiary,
                              weight: Weight.regular,
                            ),
                          ),
                        ),
                      ),
                  ],
                ),
              ),
              Container(
                decoration: BoxDecoration(
                  border: Border(
                    top: BorderSide(color: c.calendarLine),
                    left: BorderSide(color: c.calendarLine),
                  ),
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    for (var wk = 0; wk < _weeks; wk++)
                      _week(context, first.add(wk * 7), cw, entries),
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
    if (a.inline) return grid;
    return SingleChildScrollView(
      padding: const EdgeInsets.only(bottom: DbCalendar.endPadding),
      child: grid,
    );
  }

  static const _rowH = DbCalendar.row, _barH = DbCalendar.bar, _barGap = DbCalendar.barGap, _top = DbCalendar.barTop;

  Widget _week(
    BuildContext context,
    Day start,
    double cw,
    List<RangeEntry> all,
  ) {
    final c = context.colors;
    final end = start.add(6);
    final today = Day.of(clockNow());
    final week =
        [
          for (final e in all)
            if (entryStart(e).compareTo(end) <= 0 &&
                entryEnd(e).compareTo(start) >= 0)
              e,
        ]..sort((x, y) {
          final s = entryStart(x).compareTo(entryStart(y));
          if (s != 0) return s;
          return (entryEnd(y) - entryStart(y)).compareTo(
            entryEnd(x) - entryStart(x),
          );
        });
    // Lanes: each bar takes the first lane free over its days.
    final maxLanes = ((_rowH - _top - DbCalendar.barInset) / (_barH + _barGap)).floor();
    final lanes = <List<bool>>[];
    final placed = <(RangeEntry, int, int, int)>[]; // entry, lane, col, span
    final hidden = List.filled(7, 0);
    for (final e in week) {
      final s = math.max(0, entryStart(e) - start);
      final f = math.min(6, entryEnd(e) - start);
      var lane = 0;
      while (true) {
        if (lane == lanes.length) lanes.add(List.filled(7, false));
        if (!lanes[lane].sublist(s, f + 1).contains(true)) break;
        lane++;
      }
      if (lane >= maxLanes) {
        for (var d = s; d <= f; d++) {
          hidden[d]++;
        }
        continue;
      }
      for (var d = s; d <= f; d++) {
        lanes[lane][d] = true;
      }
      placed.add((e, lane, s, f - s + 1));
    }

    return SizedBox(
      height: _rowH,
      child: Stack(
        children: [
          Row(
            children: [
              for (var i = 0; i < 7; i++)
                _DayCell(
                  key: ValueKey('day-${start.add(i)}'),
                  state: this,
                  day: start.add(i),
                  width: cw,
                  today: start.add(i) == today,
                  outside: start.add(i).month != _month.month,
                  weekend:
                      (i + _weekStart) % 7 == 0 || (i + _weekStart) % 7 == 6,
                  hidden: hidden[i],
                ),
            ],
          ),
          for (final (e, lane, col, span) in placed)
            Positioned(
              left: col * cw + DbCalendar.barInset,
              top: _top + lane * (_barH + _barGap),
              width: span * cw - 2 * DbCalendar.barInset,
              height: _barH,
              child: _EntryBar(
                state: this,
                entry: e,
                cellWidth: cw,
                weekStart: start,
                col: col,
                color: eventPillFill(c, a.store.resolvedSpaceColor),
              ),
            ),
        ],
      ),
    );
  }

  Future<void> _dayList(BuildContext anchor, Day day) {
    final list = _entries(day, day);
    return showPanel<void>(
      anchor,
      globalRectOf(anchor),
      (ctx) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          MenuHeader(formatDay(day)),
          for (final e in list)
            MenuItem(
              label: displayTitle(e.page.title, firstLine: untitledLine(ctx, e.page)),
              leading: PageIcon(icon: e.page.icon, size: IconSize.md.px),
              onTap: () {
                Navigator.of(ctx).pop();
                a.ctl.openRow(context, e.page.id, title: e.page.title);
              },
            ),
        ],
      ),
      width: 260,
    );
  }

  // --------------------------------------------------------------- mobile

  Widget _mobile(BuildContext context, Day first, List<RangeEntry> entries) {
    final c = context.colors;
    final names = [
      for (var i = 0; i < 7; i++) weekdaysShort[(i + _weekStart) % 7],
    ];
    final sel = _selected ?? Day.of(clockNow());
    final today = Day.of(clockNow());
    final ofDay = _entries(sel, sel);
    return LayoutBuilder(
      builder: (context, box) {
        final cw = (box.maxWidth - 2 * a.gutter) / DateTime.daysPerWeek;
        final grid = Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          mainAxisSize: MainAxisSize.min,
          children: [
            SizedBox(
              height: DbCalendar.compactWeekdays,
              child: Row(
                children: [
                  for (final n in names)
                    SizedBox(
                      width: cw,
                      child: Center(
                        child: Text(
                          n,
                          style: ts(context.type.ui, c.textTertiary, weight: Weight.regular),
                        ),
                      ),
                    ),
                ],
              ),
            ),
            Container(height: Stroke.bold, color: c.calendarLine),
            for (var wk = 0; wk < _weeks; wk++)
              Container(
                height: DbCalendar.compactWeek,
                decoration: BoxDecoration(
                  border: Border(bottom: BorderSide(color: c.calendarLine)),
                ),
                child: Row(
                  children: [
                    for (var i = 0; i < 7; i++)
                      Builder(
                        builder: (_) {
                          final d = first.add(wk * 7 + i);
                          final n = entries
                              .where(
                                (e) =>
                                    entryStart(e).compareTo(d) <= 0 &&
                                    entryEnd(e).compareTo(d) >= 0,
                              )
                              .length;
                          return DragTarget<RangeEntry>(
                            onAcceptWithDetails: (det) => _moveTo(det.data, d),
                            builder: (context, cand, _) => GestureDetector(
                              key: ValueKey('mday-$d'),
                              behavior: HitTestBehavior.opaque,
                              onTap: () => setState(() => _selected = d),
                              child: SizedBox(
                                width: cw,
                                child: Column(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    Container(
                                      width: DbCalendar.compactDay,
                                      height: DbCalendar.compactDay,
                                      decoration: BoxDecoration(
                                        shape: BoxShape.circle,
                                        color: d == today
                                            ? c.textPrimary
                                            : (d == sel || cand.isNotEmpty)
                                            ? c.bgHover
                                            : null,
                                      ),
                                      child: Center(
                                        child: Text(
                                          '${d.day}',
                                          style: ts(
                                            context.type.body,
                                            d == today
                                                ? c.bgPrimary
                                                : (d.month != _month.month
                                                      ? c.textTertiary
                                                      : c.textPrimary),
                                            weight: d == today ? Weight.semibold : Weight.regular,
                                          ),
                                        ),
                                      ),
                                    ),
                                    const Gap(Space.xxs),
                                    SizedBox(
                                      height: DbCalendar.dotRow,
                                      child: Row(
                                        mainAxisAlignment:
                                            MainAxisAlignment.center,
                                        children: [
                                          for (
                                            var k = 0;
                                            k < math.min(3, n);
                                            k++
                                          )
                                            Container(
                                              width: DbCalendar.dot,
                                              height: DbCalendar.dot,
                                              margin:
                                                  const EdgeInsets.symmetric(
                                                    horizontal: Space.hair,
                                                  ),
                                              decoration: BoxDecoration(
                                                color: c.textTertiary,
                                                shape: BoxShape.circle,
                                              ),
                                            ),
                                        ],
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ),
                          );
                        },
                      ),
                  ],
                ),
              ),
            const Gap(Space.md),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Space.xxs),
              child: Text(
                formatDay(sel),
                style: ts(context.type.title, c.textSecondary, weight: Weight.semibold),
              ),
            ),
            const Gap(Space.xxs),
            for (final e in ofDay)
              LongPressDraggable<RangeEntry>(
                data: e,
                onDragStarted: () => _grab = 0,
                feedback: SizedBox(
                  width: DbCalendar.dragged,
                  child: _EntryChip(
                    entry: e,
                    color: eventPillFill(c, a.store.resolvedSpaceColor),
                    lifted: true,
                  ),
                ),
                child: Hover(
                  key: ValueKey('mentry-${e.page.title}'),
                  background: eventPillFill(c, a.store.resolvedSpaceColor),
                  borderRadius: Radii.pill,
                  onTap: () =>
                      a.ctl.openRow(context, e.page.id, title: e.page.title),
                  radius: Radii.md,
                  padding: const EdgeInsets.symmetric(horizontal: Space.xxs),
                  child: ConstrainedBox(
                    constraints: BoxConstraints(minHeight: ControlSize.xl.height),
                    child: Row(
                      children: [
                        PageIcon(icon: e.page.icon, size: IconSize.lg.px),
                        const Gap(Space.md),
                        Expanded(
                          child: Text(
                            displayTitle(e.page.title, firstLine: untitledLine(context, e.page)),
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: ts(context.type.body, c.textPrimary, weight: Weight.medium),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            Hover(
              key: const ValueKey('mcal-new'),
              onTap: () => _addOn(sel),
              radius: Radii.md,
              padding: const EdgeInsets.symmetric(horizontal: Space.xxs),
              child: ConstrainedBox(
                constraints: BoxConstraints(minHeight: ControlSize.xl.height),
                child: Row(
                  children: [
                    Icon20(Ico.plus, size: IconSize.md, color: c.iconTertiary),
                    const Gap(Space.md),
                    Text('New page', style: ts(context.type.body, c.textTertiary)),
                  ],
                ),
              ),
            ),
          ],
        );
        final padded = Padding(
          padding: EdgeInsets.symmetric(horizontal: a.gutter),
          child: grid,
        );
        if (a.inline) return padded;
        return SingleChildScrollView(
          padding: const EdgeInsets.only(bottom: DbCalendar.endPadding),
          child: padded,
        );
      },
    );
  }
}

class _DayCell extends StatefulWidget {
  const _DayCell({
    super.key,
    required this.state,
    required this.day,
    required this.width,
    required this.today,
    required this.outside,
    required this.weekend,
    required this.hidden,
  });
  final _CalendarViewState state;
  final Day day;
  final double width;
  final bool today, outside, weekend;
  final int hidden;

  @override
  State<_DayCell> createState() => _DayCellState();
}

class _DayCellState extends State<_DayCell> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final d = widget.day;
    final label = d.day == 1
        ? '${monthsShort[d.month - 1]} ${d.day}'
        : '${d.day}';
    final weekendBg = c.bgSecondary;
    return DragTarget<RangeEntry>(
      onAcceptWithDetails: (det) => widget.state._moveTo(det.data, d),
      builder: (context, cand, _) => MouseRegion(
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: Container(
          width: widget.width,
          height: _CalendarViewState._rowH,
          decoration: BoxDecoration(
            color: cand.isNotEmpty
                ? c.bgHover
                : (widget.weekend ? weekendBg : null),
            border: Border(
              right: BorderSide(color: c.calendarLine),
              bottom: BorderSide(color: c.calendarLine),
            ),
          ),
          child: Stack(
            children: [
              Positioned(
                right: Space.sm,
                top: Space.xxs,
                child: widget.today
                    ? Container(
                        constraints: const BoxConstraints(minWidth: DbCalendar.dayLabel, minHeight: DbCalendar.dayLabel),
                        padding: const EdgeInsets.symmetric(horizontal: Space.xs),
                        decoration: BoxDecoration(
                          color: c.textPrimary,
                          borderRadius: Radii.pill,
                        ),
                        child: Center(
                          child: Text(
                            label,
                            style: ts(context.type.ui, c.bgPrimary, weight: Weight.semibold),
                          ),
                        ),
                      )
                    : ConstrainedBox(
                        constraints: const BoxConstraints(minHeight: DbCalendar.dayLabel),
                        child: Center(
                          child: Text(
                            label,
                            style: ts(context.type.ui, widget.outside || widget.weekend ? c.textTertiary : c.textSecondary, weight: Weight.regular),
                          ),
                        ),
                      ),
              ),
              if (_hover)
                Positioned(
                  left: Space.xxs,
                  top: Space.xxs,
                  child: IconBtn(
                    Ico.plus,
                    key: ValueKey('day-add-$d'),
                    size: ControlSize.xs,
                    glyph: IconSize.xs,
                    color: c.iconTertiary,
                    semanticLabel: 'New page on ${formatDay(d)}',
                    onTap: () => widget.state._addOn(d),
                  ),
                ),
              if (widget.hidden > 0)
                Positioned(
                  left: Space.xs,
                  bottom: Space.xxs,
                  child: Builder(
                    builder: (b) => Hover(
                      onTap: () => widget.state._dayList(b, d),
                      radius: Radii.sm,
                      padding: const EdgeInsets.symmetric(horizontal: Space.xxs),
                      child: Text(
                        '${widget.hidden} more',
                        style: ts(context.type.uiSmall, c.textTertiary),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _EntryBar extends StatelessWidget {
  const _EntryBar({
    required this.state,
    required this.entry,
    required this.cellWidth,
    required this.weekStart,
    required this.col,
    required this.color,
  });
  final _CalendarViewState state;
  final RangeEntry entry;
  final double cellWidth;
  final Day weekStart;
  final int col;
  final Color color;

  @override
  Widget build(BuildContext context) {
    final a = state.a;
    final chip = _EntryChip(entry: entry, color: color);
    return Listener(
      onPointerDown: (e) {
        // The day of the bar under the pointer, counted from the entry's
        // start (so a long bar keeps its offset when dropped).
        final dayInBar = (e.localPosition.dx / cellWidth).floor();
        state._grab = weekStart.add(col + dayInBar) - entryStart(entry);
      },
      child: Draggable<RangeEntry>(
        data: entry,
        feedback: SizedBox(
          width: math.min(cellWidth - 2 * DbCalendar.barInset, DbCalendar.dragged),
          height: DbCalendar.bar,
          child: _EntryChip(entry: entry, color: color, lifted: true),
        ),
        childWhenDragging: Opacity(opacity: Alpha.wash, child: chip),
        child: GestureDetector(
          key: ValueKey('entry-${entry.page.title}'),
          onTap: () =>
              a.ctl.openRow(context, entry.page.id, title: entry.page.title),
          child: chip,
        ),
      ),
    );
  }
}

class _EntryChip extends StatelessWidget {
  const _EntryChip({
    required this.entry,
    required this.color,
    this.lifted = false,
  });
  final RangeEntry entry;
  final Color color;
  final bool lifted;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      height: DbCalendar.bar,
      padding: const EdgeInsets.symmetric(horizontal: Space.xs),
      decoration: BoxDecoration(
        color: color,
        borderRadius: Radii.pill,
        boxShadow: c.cardShadow(
          c.cardRing(null),
          lifted: lifted,
        ),
      ),
      child: Row(
        children: [
          PageIcon(icon: entry.page.icon, size: IconSize.xs.px, color: c.iconSecondary),
          const Gap(Space.xxs),
          Expanded(
            child: Text(
              displayTitle(entry.page.title, firstLine: untitledLine(context, entry.page)),
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: ts(context.type.uiSmall, c.textPrimary),
            ),
          ),
        ],
      ),
    );
  }
}
