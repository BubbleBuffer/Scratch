import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/today_timers.dart';
import '../features/daily_capture.dart';
import '../state/agenda_store.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../ui/toast.dart';
import '../ui/confetti.dart';
import '../ui/ui.dart';
import 'navigation.dart';

class TodayCapture extends StatefulWidget {
  const TodayCapture({super.key, this.mobile = false});
  final bool mobile;
  @override
  State<TodayCapture> createState() => _TodayCaptureState();
}

class _TodayCaptureState extends State<TodayCapture> {
  final _text = TextEditingController();
  late final _focus = FocusNode(
    onKeyEvent: (_, event) {
      if (event is KeyDownEvent &&
          event.logicalKey == LogicalKeyboardKey.escape) {
        _text.clear();
        return KeyEventResult.handled;
      }
      return KeyEventResult.ignored;
    },
  );
  bool _saving = false;
  String? _addedPage;
  DateTime? _addedDay;
  Timer? _feedback;

  @override
  void dispose() {
    _feedback?.cancel();
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _capture(String _) async {
    final text = _text.text.trim();
    if (_saving || text.isEmpty) return;
    setState(() => _saving = true);
    final day = calendarDay(clockNow());
    try {
      final id = await captureToday(
        context.app.vault,
        text,
        task: context.app.ui.captureTask,
      );
      if (!mounted) return;
      // Preserve anything typed while the write was in flight.
      if (_text.text.trim() == text) _text.clear();
      _feedback?.cancel();
      setState(() {
        _addedPage = id;
        _addedDay = day;
      });
      _feedback = Timer(const Duration(seconds: 4), () {
        if (mounted) setState(() => _addedPage = null);
      });
    } catch (_) {
      if (mounted) {
        showErrorToast(context, 'Could not add to today’s note. Try again.');
      }
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  Future<void> _openNote() async {
    try {
      final id = await todaysNote(context.app.vault);
      if (mounted) {
        unawaited(
          openPage(context, id, dayTitle(calendarDay(clockNow()), year: true)),
        );
      }
    } catch (_) {
      if (mounted) {
        showErrorToast(context, 'Could not open today’s note. Try again.');
      }
    }
  }

  Widget _mode(BuildContext context) {
    final task = context.app.ui.captureTask;
    final c = context.colors;
    return Container(
      key: const ValueKey('capture-mode'),
      decoration: BoxDecoration(color: c.bgSecondary, borderRadius: Radii.ctl),
      padding: const EdgeInsets.all(Space.hair),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          for (final value in [true, false])
            Semantics(
              button: true,
              selected: task == value,
              child: Hover(
                key: ValueKey(value ? 'capture-task' : 'capture-note'),
                selected: task == value,
                selectedColor: c.bgPrimary,
                onTap: () => setState(() {
                  context.app.ui
                    ..captureTask = value
                    ..changed();
                }),
                padding: const EdgeInsets.symmetric(horizontal: Space.sm, vertical: Space.sm),
                child: Text(
                  value ? 'Task' : 'Note',
                  style: ts(context.type.uiSmall, task == value ? c.textPrimary : c.textSecondary, weight: Weight.semibold),
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// "Saved to today's note · Open", or "Added to today's note · Open" for
  /// a few seconds after a capture (opening the page it went into).
  Widget _helper() {
    final c = context.colors;
    final added = _addedPage;
    return Wrap(
      key: const ValueKey('capture-helper'),
      crossAxisAlignment: WrapCrossAlignment.center,
      children: [
        Text(
          added == null ? "Saved to today's note · " : "Added to today's note · ",
          style: ts(context.type.ui, c.textSecondary, weight: Weight.regular),
        ),
        Semantics(
          link: true,
          label: "Open today's note",
          excludeSemantics: true,
          child: Hover(
            key: ValueKey(added == null ? 'todays-note' : 'capture-open'),
            onTap: added == null
                ? _openNote
                : () => openPage(
                    context,
                    added,
                    dayTitle(_addedDay!, year: true),
                  ),
            child: Text(
              'Open',
              style: ts(context.type.ui, c.accentText, weight: Weight.semibold).copyWith(decoration: TextDecoration.underline),
            ),
          ),
        ),
      ],
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final field = FieldFrame(
      height: (widget.mobile ? ControlSize.xxl : ControlSize.xl).height,
      padding: const EdgeInsets.symmetric(horizontal: Space.md),
      child: Row(
        children: [
          Expanded(
            child: PlainField(
              key: const ValueKey('capture-field'),
              controller: _text,
              focusNode: _focus,
              style: ts(context.type.ui, c.textPrimary, weight: Weight.regular),
              placeholder: 'Add a task or note…',
              onSubmitted: _capture,
              textInputAction: TextInputAction.done,
            ),
          ),
          if (widget.mobile) _mode(context),
        ],
      ),
    );
    final row = Row(
      key: const ValueKey('capture-row'),
      children: [
        Expanded(child: field),
        if (!widget.mobile) ...[const Gap(Space.xs), _mode(context)],
        const Gap(Space.sm),
        Semantics(
          button: true,
          label: 'Add to today',
          child: Hover(
            key: const ValueKey('capture-add'),
            onTap: _saving ? null : () => _capture(''),
            hoverColor: c.accentHover,
            child: Container(
              height: (widget.mobile ? ControlSize.xxl : ControlSize.xl).height,
              width: widget.mobile ? ControlSize.xxl.height : null,
              padding: EdgeInsets.symmetric(horizontal: widget.mobile ? 0 : Space.md),
              decoration: BoxDecoration(
                color: c.accent,
                borderRadius: Radii.ctl,
              ),
              child: Row(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon20(Ico.plus, size: IconSize.md, color: c.onAccent),
                  if (!widget.mobile) ...[
                    const Gap(Space.xs),
                    Text(
                      _saving ? 'Adding…' : 'Add',
                      style: ts(context.type.ui, c.onAccent, weight: Weight.bold),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
      ],
    );
    if (widget.mobile) {
      // The helper gets its own row under the capture bar.
      return Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          row,
          const Gap(Space.sm),
          Padding(
            padding: const EdgeInsets.only(left: Space.xxs, bottom: Space.xs),
            child: _helper(),
          ),
        ],
      );
    }
    return Container(
      padding: const EdgeInsets.fromLTRB(Space.lg, Space.lg, Space.lg, Space.lg),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        borderRadius: Radii.panel,
        border: Border.all(color: c.border),
        boxShadow: c.shadowSmall,
      ),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            'Add to today',
            style: tsDisplay(context.type.h3, c.textPrimary, weight: Weight.display),
          ),
          const Gap(Space.md),
          row,
          const Gap(Space.sm),
          _helper(),
        ],
      ),
    );
  }
}

class WeekStrip extends StatelessWidget {
  const WeekStrip({super.key});
  @override
  Widget build(BuildContext context) {
    final store = context.watch<AgendaStore>();
    final c = context.colors;
    return Semantics(
      label: 'Week',
      child: Row(
        key: const ValueKey('week-strip'),
        children: [
          IconBtn(
            Ico.chevronLeft,
            key: const ValueKey('week-previous'),
            onTap: () => store.moveWeek(-1),
            size: context.touch ? ControlSize.xs : ControlSize.sm,
            glyph: IconSize.sm,
            semanticLabel: 'Previous week',
          ),
          for (final day in store.days)
            Expanded(
              child: Semantics(
                button: true,
                selected: sameDay(day, store.selectedDay),
                label:
                    '${dayTitle(day)}, ${plural(store.countOn(day), 'item')}${store.overdueOn(day) ? ', overdue tasks' : ''}',
                child: Hover(
                  key: ValueKey('day-${dayStamp(day)}'),
                  onTap: () => store.selectDay(day),
                  hoverColor: c.bgPrimary,
                  lift: 2,
                  child: Container(
                    constraints: const BoxConstraints(minHeight: WeekStripSize.day),
                    margin: const EdgeInsets.symmetric(horizontal: Space.hair),
                    decoration: BoxDecoration(
                      color: sameDay(day, store.today)
                          ? c.textPrimary
                          : sameDay(day, store.selectedDay)
                          ? c.bgPrimary
                          : null,
                      borderRadius: Radii.ctl,
                      border: Border.all(
                        color:
                            sameDay(day, store.selectedDay) &&
                                !sameDay(day, store.today)
                            ? c.thread
                            : kTransparent,
                      ),
                    ),
                    child: Column(
                      mainAxisAlignment: MainAxisAlignment.center,
                      children: [
                        Text(
                          weekdays[day.weekday - 1].substring(0, 3),
                          style: ts(context.type.caption, sameDay(day, store.today) ? c.bgPrimary : c.textSecondary, weight: Weight.semibold),
                        ),
                        const Gap(Space.hair),
                        Text(
                          '${day.day}',
                          style: tsDisplay(
                            context.type.h3,
                            sameDay(day, store.today)
                                ? c.bgPrimary
                                : c.textPrimary,
                           weight: Weight.display),
                        ),
                        const Gap(Space.xs),
                        SizedBox(
                          height: WeekStripSize.dotRow,
                          child: Row(
                            mainAxisAlignment: MainAxisAlignment.center,
                            children: [
                              for (
                                var i = 0;
                                i < store.countOn(day).clamp(0, 3);
                                i++
                              )
                                Padding(
                                  padding: const EdgeInsets.symmetric(
                                    horizontal: Space.hair,
                                  ),
                                  child: Container(
                                    width: WeekStripSize.dot,
                                    height: WeekStripSize.dot,
                                    decoration: BoxDecoration(
                                      borderRadius: BorderRadius.circular(WeekStripSize.dotRadius),
                                      color: sameDay(day, store.today)
                                          ? c.accent
                                          : c.threadStrong,
                                    ),
                                  ),
                                ),
                              if (store.overdueOn(day))
                                Padding(
                                  padding: const EdgeInsets.only(left: Space.xxs),
                                  child: _dot(
                                    spaceColor('paw', Brightness.light)!,
                                    5,
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          IconBtn(
            Ico.chevronRight,
            key: const ValueKey('week-next'),
            onTap: () => store.moveWeek(1),
            size: context.touch ? ControlSize.xs : ControlSize.sm,
            glyph: IconSize.sm,
            semanticLabel: 'Next week',
          ),
        ],
      ),
    );
  }
}

/// The header's one summary line: "1 event and 1 task today.",
/// "3 tasks today, 2 overdue." or "Nothing planned today."
String todaySummary({required int events, required int tasks, required int overdue}) {
  final parts = [
    if (events > 0) plural(events, 'event'),
    if (tasks > 0) plural(tasks, 'task'),
  ];
  if (parts.isEmpty) {
    return overdue > 0 ? 'Nothing due today, $overdue overdue.' : 'Nothing planned today.';
  }
  return '${parts.join(' and ')} today${overdue > 0 ? ', $overdue overdue' : ''}.';
}

/// The line under the week strip for another selected day:
/// "Wed 7 October · 1 event, 2 tasks". Nothing when today is selected (the
/// header already summarises it).
String dayPeek(DateTime day, {required int events, required int tasks}) {
  final parts = [
    if (events > 0) plural(events, 'event'),
    if (tasks > 0) plural(tasks, 'task'),
  ];
  final title = '${weekdays[day.weekday - 1].substring(0, 3)} ${day.day} ${months[day.month - 1]}';
  return '$title · ${parts.isEmpty ? 'Nothing planned' : parts.join(', ')}';
}

class WeekPeek extends StatelessWidget {
  const WeekPeek({super.key});
  @override
  Widget build(BuildContext context) {
    final store = context.watch<AgendaStore>();
    if (store.isToday) return const SizedBox.shrink();
    final c = context.colors;
    final events = store.selectedItems
        .where((i) => i.kind == AgendaKind.event)
        .length;
    final tasks = store.selectedItems
        .where((i) => i.kind == AgendaKind.task && !i.done)
        .length;
    return Padding(
      padding: EdgeInsets.only(top: context.sizeClass.isCompact ? Space.sm : Space.xs),
      child: Text(
        dayPeek(store.selectedDay, events: events, tasks: tasks),
        key: const ValueKey('week-peek'),
        maxLines: 2,
        overflow: TextOverflow.ellipsis,
        textAlign: context.sizeClass.isCompact ? TextAlign.left : TextAlign.right,
        style: ts(context.type.ui, c.textSecondary, weight: Weight.regular),
      ),
    );
  }
}

Widget _dot(Color color, double size) => Container(
  width: size,
  height: size,
  decoration: BoxDecoration(color: color, shape: BoxShape.circle),
);
String _time(int ms) {
  final d = DateTime.fromMillisecondsSinceEpoch(ms);
  return '${d.hour.toString().padLeft(2, '0')}:${d.minute.toString().padLeft(2, '0')}';
}

class AgendaDay extends StatefulWidget {
  const AgendaDay({super.key});
  @override
  State<AgendaDay> createState() => _AgendaDayState();
}

class _DepartingTask {
  _DepartingTask(this.item, this.overdue, this.index);
  final AgendaItem item;
  final bool overdue;
  final int index;
  bool collapse = false;
}

class _AgendaDayState extends State<AgendaDay>
    with SingleTickerProviderStateMixin {
  bool _showDone = false;
  int _confettiTrigger = 0;
  int _dueLimit = 50, _overdueLimit = 50, _doneLimit = 50;
  bool _loadingOverdue = false;
  DateTime? _day;
  Timer? _minute;
  final _departing = <String, _DepartingTask>{};
  final _arriving = <String>{};
  final _taskTimers = <Timer>[];
  late final AnimationController _bump = AnimationController(vsync: this);

  @override
  void initState() {
    super.initState();
    _scheduleMinute();
  }

  void _scheduleMinute() {
    _minute = Timer(minuteRefreshDelay(clockNow()), () {
      if (!mounted) return;
      setState(() {});
      _scheduleMinute();
    });
  }

  @override
  void dispose() {
    _minute?.cancel();
    for (final timer in _taskTimers) {
      timer.cancel();
    }
    _bump.dispose();
    super.dispose();
  }

  void _open(AgendaItem i) => openPage(
    context,
    i.page,
    context.read<AgendaStore>().sourceTitles[i.page] ?? i.title,
    initialBlock: i.block,
  );

  Future<void> _toggle(AgendaItem i, {bool overdue = false}) async {
    final store = context.read<AgendaStore>();
    final motion = Motion.of(context);
    final key = AgendaStore.itemKey(i);
    final completingToday =
        store.isToday &&
        !overdue &&
        !i.done &&
        store.selectedItems
                .where((item) => item.kind == AgendaKind.task && !item.done)
                .length ==
            1;
    final selectedDay = store.selectedDay;
    // Keep a completed row briefly while the store owns the optimistic write.
    if (motion.fast != Duration.zero && !i.done) {
      final group = overdue
          ? store.overdueItems.where((item) => !item.done).toList()
          : store.selectedItems
                .where((item) => item.kind == AgendaKind.task && !item.done)
                .toList();
      final index = group.indexWhere(
        (item) => AgendaStore.itemKey(item) == key,
      );
      setState(
        () => _departing[key] = _DepartingTask(i, overdue, math.max(0, index)),
      );
      _taskTimers.add(
        Timer(motion.fast, () {
          if (!mounted || !_departing.containsKey(key)) return;
          setState(() => _departing[key]!.collapse = true);
          _taskTimers.add(
            Timer(motion.standard, () {
              if (mounted) setState(() => _departing.remove(key));
            }),
          );
        }),
      );
    } else if (motion.standard != Duration.zero && i.done) {
      setState(() => _arriving.add(key));
      _taskTimers.add(
        Timer(motion.standard, () {
          if (mounted) setState(() => _arriving.remove(key));
        }),
      );
    }
    _bump.duration = motion.standard;
    if (motion.standard == Duration.zero) {
      _bump.value = 0;
    } else {
      _bump.forward(from: 0);
    }
    final success = await store.toggleDone(i);
    if (!mounted) return;
    if (!success) {
      setState(() {
        _departing.remove(key);
        _arriving.remove(key);
      });
      if (store.error != null) showErrorToast(context, store.error!);
    } else if (completingToday &&
        store.isToday &&
        sameDay(selectedDay, store.selectedDay) &&
        store.selectedItems
            .where((item) => item.kind == AgendaKind.task)
            .every((item) => item.done)) {
      setState(() => _confettiTrigger++);
    }
  }

  Widget _chip(String label, Color background) => Container(
    padding: const EdgeInsets.fromLTRB(Space.sm, Space.xxs, Space.sm, Space.xs),
    decoration: BoxDecoration(color: background, borderRadius: Radii.pill),
    child: Text(label, style: ts(context.type.uiSmall, context.colors.textPrimary, weight: Weight.semibold)),
  );

  Widget _task(
    AgendaItem i,
    AgendaStore store, {
    bool overdue = false,
    bool departing = false,
    bool collapse = false,
  }) {
    final c = context.colors;
    // UTC calendar arithmetic avoids daylight-saving changes in late-day pills.
    final deadline = DateTime.fromMillisecondsSinceEpoch(
      (i.endMs ?? i.startMs).toInt(),
      isUtc: !i.hasTime,
    );
    final late =
        ((dayStamp(store.today) - dayStamp(deadline)) ~/
                Duration.millisecondsPerDay)
            .clamp(1, 100000);
    final checked = i.done || departing;
    final source = displayTitle(
      store.sourceTitles[AgendaStore.sourceId(i)] ?? '',
      firstLine: context.watch<PagesStore>().meta(AgendaStore.sourceId(i))?.firstLine ?? '',
    );
    final statusChip = i.status == null
        ? null
        : _chip(i.status!, switch (i.statusGroup) {
            'progress' => c.infoBg,
            'complete' => c.okBg,
            _ => c.bgSecondary,
          });
    final taskChip = overdue
        ? _chip(
            '${weekdays[deadline.weekday - 1]}, $late ${late == 1 ? 'day' : 'days'} late',
            c.dangerBg,
          )
        : statusChip;
    final metadata = context.sizeClass.isCompact
        ? taskChip
        : Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              if (source.isNotEmpty)
                Flexible(
                  child: Text(
                    source,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    textAlign: TextAlign.right,
                    style: ts(context.type.ui, c.textSecondary, weight: Weight.regular),
                  ),
                ),
              if (source.isNotEmpty && taskChip != null)
                const Gap(Space.sm),
              ?taskChip,
            ],
          );
    final row = Hover(
      key: ValueKey('agenda-${AgendaStore.itemKey(i)}'),
      onTap: () => _open(i),
      padding: const EdgeInsets.symmetric(vertical: Space.sm, horizontal: Space.hair),
      hoverColor: c.bgSecondary,
      child: Row(
        children: [
          Semantics(
            checked: checked,
            enabled: !store.busy(i) && !departing,
            label: 'Complete ${i.title}',
            child: GestureDetector(
              key: ValueKey('agenda-check-${AgendaStore.itemKey(i)}'),
              behavior: HitTestBehavior.opaque,
              onTap: store.busy(i) || departing
                  ? null
                  : () => _toggle(i, overdue: overdue),
              child: SizedBox(
                width: AgendaRow.checkTarget,
                height: AgendaRow.checkTarget,
                child: Center(
                  child: StacksCheckbox(
                    checked: checked,
                    size: AgendaRow.check,
                    fill: spacePatch(
                      i.spaceColor,
                      c.dark ? Brightness.dark : Brightness.light,
                    )?.fill,
                    ink: spacePatch(
                      i.spaceColor,
                      c.dark ? Brightness.dark : Brightness.light,
                    )?.ink,
                  ),
                ),
              ),
            ),
          ),
          const Gap(Space.sm),
          Expanded(
            child: Text(
              i.title,
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style:
                  ts(context.type.body, checked ? c.textSecondary : c.textPrimary).copyWith(
                    decoration: checked ? TextDecoration.lineThrough : null,
                  ),
            ),
          ),
          if (metadata != null) ...[
            const Gap(Space.sm),
            ConstrainedBox(
              constraints: BoxConstraints(maxWidth: context.sizeClass.isCompact ? AgendaRow.metadataCompact : AgendaRow.metadata),
              child: metadata,
            ),
          ],
        ],
      ),
    );
    final arriving = _arriving.contains(AgendaStore.itemKey(i));
    return TweenAnimationBuilder<double>(
      key: ValueKey(
        'task-motion-${AgendaStore.itemKey(i)}${arriving ? '-arrive' : ''}',
      ),
      tween: Tween(begin: arriving ? 0 : 1, end: collapse ? 0 : 1),
      duration: Motion.of(context).standard,
      curve: Motion.ease,
      child: row,
      builder: (context, value, child) => ClipRect(
        child: Align(
          heightFactor: value,
          alignment: Alignment.topCenter,
          child: Opacity(opacity: value, child: child),
        ),
      ),
    );
  }

  List<Widget> _taskGroup(
    List<AgendaItem> items,
    AgendaStore store,
    int limit, {
    bool overdue = false,
  }) {
    final rows = [
      for (final item in items.take(limit))
        _task(item, store, overdue: overdue),
    ];
    final leaving =
        _departing.values.where((task) => task.overdue == overdue).toList()
          ..sort((a, b) => a.index.compareTo(b.index));
    for (final task in leaving) {
      rows.insert(
        math.min(task.index, rows.length),
        _task(
          task.item,
          store,
          overdue: overdue,
          departing: true,
          collapse: task.collapse,
        ),
      );
    }
    return rows;
  }

  Widget _allDay(AgendaItem i) => Hover(
    key: ValueKey('agenda-${AgendaStore.itemKey(i)}'),
    onTap: () => _open(i),
    child: _chip(i.title, context.colors.bgSecondary),
  );

  Widget _timeline(List<AgendaItem> items, AgendaStore store) {
    final c = context.colors;
    final nowMs = clockNow().millisecondsSinceEpoch;
    final midnight = calendarDay(store.selectedDay).millisecondsSinceEpoch;
    double hour(AgendaItem i) =>
        (i.startMs.toInt() - midnight) / Duration.millisecondsPerHour;
    final start = math.max(7.0, math.min(21.0, hour(items.first) - 1));
    final end = math.max(
      start + 1,
      math.min(22.0, math.max(8.0, hour(items.last) + 1)),
    );
    final next = store.isToday
        ? items.indexWhere((i) => i.startMs.toInt() > nowMs)
        : -1;
    bool past(AgendaItem item) =>
        store.isToday && (item.endMs ?? item.startMs).toInt() < nowMs;
    String timeRange(AgendaItem item) =>
        '${_time(item.startMs.toInt())}${item.endMs == null ? '' : '–${_time(item.endMs!.toInt())}'}';
    return LayoutBuilder(
      builder: (context, constraints) {
        // Separate close nodes and give their labels extra lanes. Very busy
        // days can scroll the track rather than obscure clickable events.
        final width = math.max(constraints.maxWidth, items.length * DayTimeline.perItem + Space.xl);
        final laneEnds = <double>[];
        final positions =
            <
              ({
                double node,
                double label,
                double width,
                int lane,
                bool countdownLine,
              })
            >[];
        final nodes = [
          for (final item in items)
            ((hour(item) - start) / (end - start) * width)
                .clamp(DayTimeline.nodeInset, width - DayTimeline.nodeInset)
                .toDouble(),
        ];
        for (var index = 1; index < nodes.length; index++) {
          nodes[index] = math.max(nodes[index], nodes[index - 1] + DayTimeline.nodeGap);
        }
        nodes[nodes.length - 1] = math.min(nodes.last, width - DayTimeline.nodeInset);
        for (var index = nodes.length - 2; index >= 0; index--) {
          nodes[index] = math.min(nodes[index], nodes[index + 1] - DayTimeline.nodeGap);
        }
        double textWidth(String text, TextStyle style) {
          final painter = TextPainter(
            text: TextSpan(text: text, style: style),
            textDirection: TextDirection.ltr,
            textScaler: MediaQuery.textScalerOf(context),
          )..layout();
          return painter.width;
        }

        final timeStyle = ts(context.type.codeSmall, c.textSecondary, mono: true);
        for (var index = 0; index < nodes.length; index++) {
          final node = nodes[index];
          final available =
              math.min(
                index == 0 ? width : node - nodes[index - 1],
                index == nodes.length - 1 ? width : nodes[index + 1] - node,
              ) -
              8;
          var labelWidth = math
              .max(
                88.0,
                math.max(
                  textWidth(
                        items[index].title,
                        ts(context.type.uiSmall, c.textPrimary, weight: Weight.bold),
                      ) +
                      8,
                  textWidth(timeRange(items[index]), timeStyle) + 8,
                ),
              )
              .clamp(88.0, math.min(240.0, width))
              .toDouble();
          var countdownLine = false;
          if (next == index && !context.sizeClass.isCompact) {
            final fullWidth =
                textWidth(
                  '${timeRange(items[index])} · ${_until(items[index].startMs.toInt() - nowMs)}',
                  timeStyle,
                ) +
                8;
            if (fullWidth <= available) {
              labelWidth = math.max(labelWidth, fullWidth);
            } else {
              countdownLine = true;
              labelWidth = math.max(
                labelWidth,
                textWidth(
                      _until(items[index].startMs.toInt() - nowMs),
                      timeStyle,
                    ) +
                    8,
              );
            }
          }
          final left = (node - labelWidth / 2).clamp(
            0.0,
            math.max(0.0, width - labelWidth),
          );
          var lane = laneEnds.indexWhere((right) => left >= right + 8);
          if (lane < 0) {
            lane = laneEnds.length;
            laneEnds.add(0);
          }
          laneEnds[lane] = left + labelWidth;
          positions.add((
            node: node.toDouble(),
            label: left.toDouble(),
            width: labelWidth,
            lane: lane,
            countdownLine: countdownLine,
          ));
        }
        final nowHour = (nowMs - midnight) / Duration.millisecondsPerHour;
        // Align the two-pixel marker and elapsed track to whole pixels.
        final nowX = ((nowHour - start) / (end - start) * width)
            .clamp(0.0, width)
            .roundToDouble();
        final timeline = Padding(
          padding: const EdgeInsets.only(top: Space.md, bottom: Space.lg),
          child: SizedBox(
            height:
                (positions.any((p) => p.countdownLine) ? DayTimeline.countdownHeight : DayTimeline.height) +
                math.max(0, laneEnds.length - 1) * DayTimeline.lane,
            child: Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned(
                  top: DayTimeline.trackTop,
                  left: 0,
                  right: 0,
                  child: Container(
                    height: DayTimeline.track,
                    decoration: BoxDecoration(
                      color: c.bgSecondary,
                      borderRadius: BorderRadius.circular(Radii.sm),
                    ),
                  ),
                ),
                if (store.isToday)
                  Positioned(
                    top: DayTimeline.trackTop,
                    left: 0,
                    width: nowX,
                    child: Container(
                      height: DayTimeline.track,
                      decoration: BoxDecoration(
                        color: c.thread.withValues(alpha: Alpha.muted),
                        borderRadius: BorderRadius.circular(Radii.sm),
                      ),
                    ),
                  ),
                if (store.isToday)
                  Positioned(
                    key: const ValueKey('agenda-now'),
                    left: nowX,
                    top: DayTimeline.nowTop,
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Transform.translate(
                          offset: Offset(nowX > width - DayTimeline.nowFlipZone ? -DayTimeline.nowFlip : DayTimeline.nowLabelGap, -DayTimeline.nowLift),
                          child: Text(
                            'now ${_time(nowMs)}',
                            style: ts(context.type.uiSmall, c.accentText, weight: Weight.bold),
                          ),
                        ),
                        Container(width: Stroke.bold, height: DayTimeline.nowLine, color: c.accentText),
                      ],
                    ),
                  ),
                for (var index = 0; index < items.length; index++) ...[
                  Positioned(
                    left: positions[index].node - DayTimeline.nodeInset,
                    top: DayTimeline.nodeTop,
                    child: Hover(
                      key: ValueKey(
                        'agenda-${AgendaStore.itemKey(items[index])}',
                      ),
                      onTap: () => _open(items[index]),
                      child: CustomPaint(
                        painter: _EventNodePainter(
                          next == index ? c.accent : c.bgPrimary,
                          past(items[index]) ? c.thread : c.threadStrong,
                          past: past(items[index]),
                          next: next == index,
                        ),
                        child: SizedBox(
                          width: DayTimeline.node,
                          height: DayTimeline.node,
                          child: Center(
                            child: past(items[index])
                                ? Icon20(
                                    Ico.check,
                                    size: IconSize.xs,
                                    color: c.textPrimary,
                                  )
                                : null,
                          ),
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    left: positions[index].label,
                    top: DayTimeline.labelTop + positions[index].lane * DayTimeline.lane,
                    width: positions[index].width,
                    child: GestureDetector(
                      onTap: () => _open(items[index]),
                      child: Column(
                        children: [
                          Text(
                            items[index].title,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            textAlign: TextAlign.center,
                            style: ts(context.type.uiSmall, past(items[index]) ? c.textSecondary : c.textPrimary, weight: Weight.bold),
                          ),
                          Text(
                            '${timeRange(items[index])}${next == index && !context.sizeClass.isCompact && !positions[index].countdownLine ? ' · ${_until(items[index].startMs.toInt() - nowMs)}' : ''}',
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: timeStyle,
                          ),
                          if (positions[index].countdownLine)
                            Text(
                              _until(items[index].startMs.toInt() - nowMs),
                              maxLines: 1,
                              style: timeStyle,
                            ),
                        ],
                      ),
                    ),
                  ),
                ],
                if (store.isToday)
                  Positioned(
                    left: nowX - Sizes.focusGap,
                    top: DayTimeline.nodeTop,
                    child: _dot(c.accentText, DayTimeline.nowDot),
                  ),
              ],
            ),
          ),
        );
        return width > constraints.maxWidth
            ? SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: SizedBox(width: width, child: timeline),
              )
            : timeline;
      },
    );
  }

  String _until(int ms) {
    final minutes = math.max(0, ms ~/ Duration.millisecondsPerMinute);
    return minutes >= 60
        ? 'in ${minutes ~/ 60} h ${minutes % 60}'
        : 'in $minutes min';
  }

  Widget _label(String text) => Padding(
    padding: const EdgeInsets.only(top: Space.md, bottom: Space.xxs),
    child: Text(text, style: ts(context.type.ui, context.colors.textSecondary, weight: Weight.bold)),
  );
  Widget _more(String group, int remaining, VoidCallback? onTap) => Align(
    alignment: Alignment.centerLeft,
    child: TextBtn(
      'Show $remaining more',
      key: ValueKey('agenda-more-$group'),
      onTap: onTap,
      color: context.colors.textSecondary,
    ),
  );
  Future<void> _moreOverdue(AgendaStore store) async {
    if (_loadingOverdue) return;
    final day = store.selectedDay;
    setState(() => _loadingOverdue = true);
    try {
      await store.showMoreOverdue();
      if (mounted && sameDay(day, store.selectedDay)) {
        setState(() {
          if (store.error == null) _overdueLimit += 50;
        });
      }
    } finally {
      if (mounted) setState(() => _loadingOverdue = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<AgendaStore>();
    final c = context.colors;
    if (_day == null || !sameDay(_day!, store.selectedDay)) {
      _showDone = false;
      _dueLimit = _overdueLimit = _doneLimit = 50;
      _departing.clear();
      _arriving.clear();
      _day = store.selectedDay;
    }
    final events = store.selectedItems
        .where((i) => i.kind == AgendaKind.event)
        .toList();
    final allDay = events.where((i) => !i.hasTime).toList();
    final timed = events.where((i) => i.hasTime).toList()
      ..sort((a, b) => a.startMs.compareTo(b.startMs));
    final due = store.selectedItems
        .where((i) => i.kind == AgendaKind.task && !i.done)
        .toList();
    final done = store.selectedDoneItems;
    final dueTasks = store.selectedItems
        .where((item) => item.kind == AgendaKind.task)
        .toList();
    final allDone =
        store.isToday &&
        dueTasks.isNotEmpty &&
        dueTasks.every((item) => item.done);
    final overdue = store.isToday
        ? store.overdueItems.where((i) => !i.done).toList()
        : <AgendaItem>[];
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Wrap(
          spacing: Space.md,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Text(
              store.isToday ? 'Today' : dayTitle(store.selectedDay),
              style: tsDisplay(context.type.h2, c.textPrimary, weight: Weight.display),
            ),
            if (!store.isToday)
              TextBtn(
                'Back to today',
                key: const ValueKey('back-today'),
                onTap: store.backToToday,
                color: c.accentText,
              ),
          ],
        ),
        if (store.loading && store.items.isEmpty)
          Text('Loading…', style: ts(context.type.ui, c.textSecondary)),
        if (store.error != null)
          Row(
            children: [
              Expanded(
                child: Text(
                  store.error!,
                  style: ts(context.type.ui, c.textSecondary),
                ),
              ),
              TextBtn('Retry', onTap: store.reload),
            ],
          ),
        if (allDay.isNotEmpty) ...[
          const Gap(Space.md),
          Wrap(
            spacing: Space.xs,
            runSpacing: Space.xs,
            children: [for (final item in allDay) _allDay(item)],
          ),
        ],
        if (timed.isNotEmpty) _timeline(timed, store),
        if (store.isToday &&
            (store.overdueCount > 0 ||
                _departing.values.any((task) => task.overdue))) ...[
          _label('Overdue · ${store.overdueCount}'),
          ..._taskGroup(overdue, store, _overdueLimit, overdue: true),
          if (store.overdueCount > _overdueLimit)
            _more(
              'overdue',
              store.overdueCount - _overdueLimit,
              _loadingOverdue ? null : () => _moreOverdue(store),
            ),
        ],
        if (due.isNotEmpty ||
            _departing.values.any((task) => !task.overdue)) ...[
          _label('Due · ${due.length}'),
          ..._taskGroup(due, store, _dueLimit),
          if (due.length > _dueLimit)
            _more(
              'due',
              due.length - _dueLimit,
              () => setState(() => _dueLimit += 50),
            ),
        ],
        if (!store.loading &&
            events.isEmpty &&
            due.isEmpty &&
            done.isEmpty &&
            overdue.isEmpty)
          Padding(
            padding: const EdgeInsets.only(top: Space.lg),
            child: Text(
              'Nothing planned. Add a task above, or give any to-do a date with @.',
              style: ts(context.type.ui, c.textSecondary, weight: Weight.regular),
            ),
          ),
        // The done pile shows once something is done.
        if (done.isNotEmpty || _departing.isNotEmpty) ...[
        const Gap(Space.md),
        Container(height: Stroke.hair, color: c.border),
        const Gap(Space.md),
        Row(
          key: const ValueKey('agenda-done-row'),
          children: [
            const Spacer(),
            AnimatedBuilder(
              animation: _bump,
              builder: (context, child) => Transform.scale(
                scale: 1 + math.sin(_bump.value * math.pi) * DonePile.bump,
                child: child,
              ),
              child: ConfettiBurst(
                key: const ValueKey('agenda-confetti'),
                trigger: _confettiTrigger,
                patch: spacePatch(
                  'apricot',
                  c.dark ? Brightness.dark : Brightness.light,
                )!,
                child: Hover(
                  key: const ValueKey('agenda-done'),
                  onTap: () => setState(() => _showDone = !_showDone),
                  padding: const EdgeInsets.symmetric(
                    horizontal: Space.xs,
                    vertical: Space.xs,
                  ),
                  child: Row(
                    children: [
                      SizedBox(
                        width: DonePile.width,
                        height: DonePile.height,
                        child: Stack(
                          children: [
                            for (var i = 0; i < math.min(done.length, 4); i++)
                              Positioned(
                                left: i.isEven ? DonePile.slipInset : DonePile.slipInsetOdd,
                                right: i.isEven ? DonePile.slipInsetOdd : DonePile.slipInset,
                                bottom: i * DonePile.slipStep,
                                child: Container(
                                  height: DonePile.slip,
                                  decoration: BoxDecoration(
                                    color: c.okBg,
                                    border: Border.all(color: c.thread),
                                    borderRadius: Radii.ctl,
                                  ),
                                ),
                              ),
                          ],
                        ),
                      ),
                      const Gap(Space.xs),
                      Text(
                        allDone
                            ? 'All done'
                            : '${done.length} done${store.isToday ? ' today' : ''}',
                        style: ts(context.type.ui, c.textSecondary, weight: Weight.bold),
                      ),
                      const Gap(Space.xs),
                      Icon20(
                        _showDone ? Ico.chevronDown : Ico.chevronRight,
                        size: IconSize.xs,
                        color: c.iconSecondary,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
        ],
        if (_showDone && done.isNotEmpty) ...[
          for (final i in done.take(_doneLimit))
            if (!_departing.containsKey(AgendaStore.itemKey(i)))
              _task(i, store),
          if (done.length > _doneLimit)
            _more(
              'done',
              done.length - _doneLimit,
              () => setState(() => _doneLimit += 50),
            ),
        ],
      ],
    );
    return Padding(
      padding: const EdgeInsets.only(bottom: Space.md),
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          Positioned(
            left: DonePile.backLeft,
            right: DonePile.backRight,
            bottom: -DonePile.backDrop,
            height: DonePile.card,
            child: Container(
              decoration: BoxDecoration(
                color: c.bgPrimary.withValues(alpha: Alpha.strong),
                border: Border.all(color: c.border),
                borderRadius: Radii.panel,
              ),
            ),
          ),
          Positioned(
            left: DonePile.frontInset,
            right: DonePile.frontInset,
            bottom: -DonePile.frontDrop,
            height: DonePile.card,
            child: Container(
              decoration: BoxDecoration(
                color: c.bgPrimary,
                border: Border.all(color: c.border),
                borderRadius: Radii.panel,
              ),
            ),
          ),
          Container(
            padding: EdgeInsets.fromLTRB(
              context.sizeClass.isCompact ? Space.lg : Space.xl,
              Space.xl,
              context.sizeClass.isCompact ? Space.lg : Space.xl,
              Space.lg,
            ),
            decoration: BoxDecoration(
              color: c.bgPrimary,
              border: Border.all(color: c.border),
              borderRadius: Radii.panel,
              boxShadow: c.shadowSmall,
            ),
            child: content,
          ),
        ],
      ),
    );
  }
}

class _EventNodePainter extends CustomPainter {
  const _EventNodePainter(
    this.fill,
    this.border, {
    required this.past,
    required this.next,
  });
  final Color fill, border;
  final bool past, next;
  @override
  void paint(Canvas canvas, Size size) {
    final shape = Radii.ctl.toRRect(Offset.zero & size).deflate(DayTimeline.nodeRingInset);
    canvas.drawRRect(shape, Paint()..color = fill);
    final paint = Paint()
      ..color = next ? fill : border
      ..strokeWidth = Stroke.regular
      ..style = PaintingStyle.stroke;
    if (past) {
      final path = Path()..addRRect(shape);
      for (final metric in path.computeMetrics()) {
        for (double distance = 0; distance < metric.length; distance += DayTimeline.dash) {
          canvas.drawPath(
            metric.extractPath(distance, math.min(distance + 3, metric.length)),
            paint,
          );
        }
      }
    } else {
      canvas.drawRRect(shape, paint);
    }
  }

  @override
  bool shouldRepaint(_EventNodePainter old) =>
      old.fill != fill ||
      old.border != border ||
      old.past != past ||
      old.next != next;
}

class InMotion extends StatelessWidget {
  const InMotion({super.key});
  @override
  Widget build(BuildContext context) {
    final items = context.watch<AgendaStore>().inMotion;
    if (items.isEmpty) return const SizedBox.shrink();
    final c = context.colors;
    return Column(
      key: const ValueKey('in-motion'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          'In progress',
          style: tsDisplay(context.type.h3, c.textPrimary, weight: Weight.display),
        ),
        const Gap(Space.md),
        for (final item in items)
          Hover(
            key: ValueKey('motion-${item.page}'),
            onTap: () => openPage(context, item.page, item.title),
            hoverColor: c.bgPrimary,
            padding: const EdgeInsets.symmetric(vertical: Space.sm, horizontal: Space.sm),
            child: Row(
              children: [
                SizedBox(
                  width: MotionRing.size,
                  height: MotionRing.size,
                  child: CustomPaint(
                    painter: _ProgressRingPainter(
                      item.total == 0 ? 0 : item.done / item.total,
                      c.border,
                      item.done == item.total
                          ? c.accent
                          : spaceColor(
                                  item.spaceColor,
                                  c.dark ? Brightness.dark : Brightness.light,
                                ) ??
                                spaceColor('moss', Brightness.light)!,
                    ),
                    child: Center(
                      child: Text(
                        '${item.done}/${item.total}',
                        style: tsDisplay(
                          context.type.uiSmall,
                          c.textPrimary,
                         weight: Weight.display),
                      ),
                    ),
                  ),
                ),
                const Gap(Space.lg),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.stretch,
                    children: [
                      Text(
                        item.title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: ts(context.type.title, c.textPrimary, weight: Weight.bold),
                      ),
                      Text(
                        '${item.done} of ${item.total} done',
                        style: ts(context.type.ui, c.textSecondary, weight: Weight.regular),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

class _ProgressRingPainter extends CustomPainter {
  const _ProgressRingPainter(this.progress, this.track, this.fill);
  final double progress;
  final Color track, fill;
  @override
  void paint(Canvas canvas, Size size) {
    final rect = (Offset.zero & size).deflate(MotionRing.inset);
    final paint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeWidth = MotionRing.stroke
      ..strokeCap = StrokeCap.round
      ..color = track;
    canvas.drawOval(rect, paint);
    paint.color = fill;
    canvas.drawArc(
      rect,
      -math.pi / 2,
      math.pi * 2 * progress.clamp(0, 1),
      false,
      paint,
    );
  }

  @override
  bool shouldRepaint(_ProgressRingPainter old) =>
      old.progress != progress || old.track != track || old.fill != fill;
}
