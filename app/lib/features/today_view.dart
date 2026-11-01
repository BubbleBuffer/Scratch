import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../addons/tasks/api.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../services/addon_services.dart';
import '../state/agenda_store.dart';
import '../state/app_state.dart';
import '../ui/toast.dart';
import '../ui/ui.dart';
import 'today_date_strip.dart';

/// Today's selected-day workspace. The shell supplies the ordinary day-note
/// editor in [note], keeping its document lifetime separate from the agenda.
class TodayView extends StatefulWidget {
  const TodayView({super.key, this.note, this.includeStrip = true});
  final Widget? note;
  final bool includeStrip;

  @override
  State<TodayView> createState() => _TodayViewState();
}

class _TodayViewState extends State<TodayView> {
  final _debounce = Debounce();
  AppState? _app;
  DateTime? _day;
  CalendarSettings? _calendars;
  final Map<String, TaskData> _tasks = {};
  int _generation = 0;
  // Today's open and done rows share _limit; overdue rows have their own.
  int _limit = 5;
  int _overdueLimit = 5;
  bool _overdueOpen = true;
  bool _doneOpen = false;
  bool _loadingOverdue = false;
  bool _creating = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_app != null) return;
    _app = context.app;
    _app!.feed.addListener(_changed);
    _app!.agenda.addListener(_agendaChanged);
    unawaited(_loadPresentation());
  }

  void _changed(ChangeBatch _) {
    _tasks.clear();
    _generation++;
    _debounce.run(
      const Duration(milliseconds: 150),
      () => unawaited(_loadPresentation()),
    );
  }

  void _agendaChanged() => _debounce.run(
    const Duration(milliseconds: 50),
    () => unawaited(_loadPresentation()),
  );

  Future<void> _loadPresentation() async {
    final app = _app;
    if (!mounted || app == null || app.vault.isClosed) return;
    final generation = ++_generation;
    try {
      final calendars = await app.vault.listCalendars();
      final tasks = <String, TaskData>{};
      final items = [
        ...app.agenda.selectedItems,
        ...app.agenda.selectedDoneItems,
        if (app.agenda.isToday) ...app.agenda.overdueItems,
      ];
      final pages = {
        for (final item in items)
          if (item.kind == AgendaKind.task &&
              item.block == null &&
              item.collection == null)
            item.page,
      };
      await Future.wait([
        for (final page in pages)
          if (!_tasks.containsKey(page))
            () async {
              try {
                tasks[page] = await loadTask(app.vault, page);
              } catch (_) {
                tasks[page] = const TaskData();
              }
            }(),
      ]);
      if (!mounted || app.vault.isClosed || generation != _generation) return;
      setState(() {
        _calendars = calendars;
        _tasks.addAll(tasks);
      });
    } catch (_) {
      // Source colours fall back to the item's space while categories reload.
      // AgendaStore owns the actionable load error and retry for the items.
    }
  }

  Future<void> _toggle(AgendaItem item) async {
    final store = _app!.agenda;
    final success = await store.toggleDone(item);
    if (mounted && !success && store.error != null) {
      showErrorToast(context, store.error!);
    }
  }

  Future<void> _create() async {
    if (_creating) return;
    _creating = true;
    try {
      await showTaskComposer(
        context,
        date: _app!.agenda.selectedDay,
        fromToday: true,
      );
    } finally {
      _creating = false;
    }
  }

  Future<void> _moreOverdue(AgendaStore store, {required bool fetch}) async {
    if (_loadingOverdue) return;
    if (!fetch) {
      setState(() => _overdueLimit += 5);
      return;
    }
    final day = store.selectedDay;
    setState(() => _loadingOverdue = true);
    try {
      await store.showMoreOverdue();
      if (mounted && sameDay(day, store.selectedDay) && store.error == null) {
        setState(() => _overdueLimit += 5);
      }
    } finally {
      if (mounted) setState(() => _loadingOverdue = false);
    }
  }

  // AgendaStore's completion projection currently omits these two fields.
  // Restore presentation metadata from the task read already used for its
  // optional checkbox, keeping completion and all scheduling fields store-owned.
  AgendaItem _presentationItem(AgendaItem item) {
    final task = _tasks[item.page];
    if (task == null || item.block != null || item.collection != null) {
      return item;
    }
    final calendar = item.calendar ?? task.calendar;
    final assignee = item.assignee ?? task.assignee;
    if (calendar == item.calendar && assignee == item.assignee) return item;
    return AgendaItem(
      page: item.page,
      block: item.block,
      title: item.title,
      kind: item.kind,
      startMs: item.startMs,
      endMs: item.endMs,
      hasTime: item.hasTime,
      done: item.done,
      status: item.status,
      statusGroup: item.statusGroup,
      statusColor: item.statusColor,
      calendar: calendar,
      assignee: assignee,
      collection: item.collection,
      space: item.space,
      spaceColor: item.spaceColor,
      icon: item.icon,
      updatedMs: item.updatedMs,
    );
  }

  Widget _card(AgendaItem item, AgendaStore store, {bool overdue = false}) =>
      Padding(
        padding: const EdgeInsets.only(bottom: 3),
        child: ShortItemCard(
          key: ValueKey('today-item-${AgendaStore.itemKey(item)}'),
          item: _presentationItem(item),
          calendars: _calendars,
          sourceTitle: store.sourceTitles[AgendaStore.sourceId(item)],
          overdue: overdue,
          checkbox: _tasks[item.page]?.checkbox ?? true,
          busy: store.busy(item),
          onToggle: () => unawaited(_toggle(item)),
          onOpen: () => openPage(
            context,
            item.page,
            store.sourceTitles[item.page] ?? item.title,
            initialBlock: item.block,
          ),
        ),
      );

  Widget _disclosure(
    String label,
    int count,
    bool open,
    Key key,
    VoidCallback toggle,
  ) {
    final c = context.colors;
    return Semantics(
      button: true,
      expanded: open,
      child: Hover(
        key: key,
        onTap: toggle,
        lift: 0,
        padding: const EdgeInsets.symmetric(vertical: 6),
        child: Row(
          children: [
            Icon20(
              open ? Ico.chevronDown : Ico.chevronRight,
              size: 14,
              color: c.iconSecondary,
            ),
            const SizedBox(width: 4),
            Text('$label · ', style: tsRaw(12, 18, 600, c.textSecondary)),
            Text('$count', style: tsRaw(12, 18, 400, c.textTertiary)),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<AgendaStore>();
    final c = context.colors;
    if (_day == null || !sameDay(_day!, store.selectedDay)) {
      _day = store.selectedDay;
      _limit = 5;
      _overdueLimit = 5;
      // Overdue opens again each day; a collapse applies to that day only.
      _overdueOpen = true;
      _doneOpen = false;
    }
    // Calendar can clip an event across days. Today's compact agenda follows
    // its start date, and its core query already applies Show in Today.
    final selected = store.selectedItems
        .where(
          (item) =>
              item.kind != AgendaKind.mention &&
              sameDay(store.itemDay(item), store.selectedDay),
        )
        .toList();
    final open = selected.where((item) => !item.done).toList()
      ..sort((a, b) {
        if (a.hasTime != b.hasTime) return a.hasTime ? -1 : 1;
        final order = a.startMs.compareTo(b.startMs);
        return order == 0
            ? AgendaStore.itemKey(a).compareTo(AgendaStore.itemKey(b))
            : order;
      });
    final done = store.selectedDoneItems
        .where(
          (item) =>
              item.kind != AgendaKind.mention &&
              (sameDay(store.itemDay(item), store.selectedDay) ||
                  store.isToday),
        )
        .toList();
    final overdue = store.isToday
        ? store.overdueItems
              .where((item) => !item.done && item.kind != AgendaKind.mention)
              .toList()
        : <AgendaItem>[];
    final doneVisible = done.length <= 2 || _doneOpen;
    var budget = _limit;
    List<AgendaItem> take(List<AgendaItem> items) {
      final visible = items.take(math.max(0, budget)).toList();
      budget -= visible.length;
      return visible;
    }

    // Overdue rows take their own budget, so they never push out Today's rows.
    final shownOverdue = _overdueOpen
        ? overdue.take(math.max(0, _overdueLimit)).toList()
        : <AgendaItem>[];
    final overdueHidden = _overdueOpen && store.isToday
        ? math.max(0, store.overdueCount - shownOverdue.length)
        : 0;
    final fetchOverdue =
        _overdueOpen &&
        store.overdueCount > overdue.length &&
        _overdueLimit + 5 > overdue.length;
    final shownOpen = take(open);
    final shownDone = doneVisible ? take(done) : <AgendaItem>[];
    final shown = shownOpen.length + shownDone.length;
    final total = open.length + (doneVisible ? done.length : 0);
    final hidden = math.max(0, total - shown);
    final shortDay =
        '${weekdays[store.selectedDay.weekday - 1].substring(0, 3)} '
        '${store.selectedDay.day} ${months[store.selectedDay.month - 1].substring(0, 3)}';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (widget.includeStrip) const TodayDateStrip(),
        const SizedBox(height: 14),
        Row(
          children: [
            Expanded(
              child: Wrap(
                spacing: 10,
                crossAxisAlignment: WrapCrossAlignment.center,
                children: [
                  Text(
                    shortDay,
                    key: const ValueKey('today-selected-heading'),
                    style: tsDisplay(
                      const TypeSpec(21, 25, 640),
                      c.textPrimary,
                    ),
                  ),
                  if (store.isToday)
                    Text('Today', style: tsRaw(12, 18, 500, c.accentText)),
                ],
              ),
            ),
            if (const AddonServices().addonEnabled('tasks'))
              TextBtn(
                'Add task',
                key: const ValueKey('today-add-task'),
                onTap: _create,
                color: c.textSecondary,
              ),
          ],
        ),
        const SizedBox(height: 8),
        if (store.loading && store.items.isEmpty)
          Text(
            'Loading…',
            key: const ValueKey('today-loading'),
            style: ts(context.type.uiSmall, c.textSecondary),
          ),
        if (store.error != null)
          Row(
            children: [
              Expanded(
                child: Text(
                  store.error!,
                  style: ts(context.type.uiSmall, c.textSecondary),
                ),
              ),
              TextBtn(
                'Retry',
                key: const ValueKey('today-retry'),
                onTap: store.reload,
              ),
            ],
          ),
        if (store.isToday && store.overdueCount > 0) ...[
          _disclosure(
            'Overdue',
            store.overdueCount,
            _overdueOpen,
            const ValueKey('today-overdue'),
            () => setState(() => _overdueOpen = !_overdueOpen),
          ),
          for (final item in shownOverdue) _card(item, store, overdue: true),
          if (overdueHidden > 0)
            Align(
              alignment: Alignment.centerLeft,
              child: TextBtn(
                _loadingOverdue ? 'Loading…' : 'Show $overdueHidden more',
                key: const ValueKey('today-overdue-more'),
                onTap: _loadingOverdue
                    ? null
                    : () => _moreOverdue(store, fetch: fetchOverdue),
                color: c.accentText,
              ),
            )
          else if (_overdueLimit > 5 && shownOverdue.length > 5)
            Align(
              alignment: Alignment.centerLeft,
              child: TextBtn(
                'Show less',
                key: const ValueKey('today-overdue-less'),
                onTap: () => setState(() => _overdueLimit = 5),
                color: c.accentText,
              ),
            ),
          const SizedBox(height: 6),
        ],
        for (final item in shownOpen) _card(item, store),
        if (!store.loading && selected.isEmpty && done.isEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(0, 6, 0, 16),
            child: Text(
              'Nothing scheduled',
              key: const ValueKey('today-empty'),
              style: ts(context.type.uiSmall, c.textTertiary),
            ),
          ),
        if (done.isNotEmpty) ...[
          if (done.length > 2)
            _disclosure(
              'Done',
              done.length,
              _doneOpen,
              const ValueKey('today-done'),
              () => setState(() => _doneOpen = !_doneOpen),
            )
          else
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 6),
              child: Text(
                'Done · ${done.length}',
                key: const ValueKey('today-done'),
                style: tsRaw(12, 18, 600, c.textSecondary),
              ),
            ),
          for (final item in shownDone) _card(item, store),
        ],
        if (hidden > 0)
          Align(
            alignment: Alignment.centerLeft,
            child: TextBtn(
              'Show $hidden more',
              key: const ValueKey('today-more'),
              onTap: () => setState(() => _limit += 5),
              color: c.accentText,
            ),
          )
        else if (_limit > 5 && shown > 5)
          Align(
            alignment: Alignment.centerLeft,
            child: TextBtn(
              'Show less',
              key: const ValueKey('today-less'),
              onTap: () => setState(() => _limit = 5),
              color: c.accentText,
            ),
          ),
        if (widget.note != null) ...[
          const SizedBox(height: 14),
          Container(height: 1, color: c.border),
          const SizedBox(height: 14),
          widget.note!,
        ],
      ],
    );
  }

  @override
  void dispose() {
    _generation++;
    _debounce.cancel();
    _app?.feed.removeListener(_changed);
    _app?.agenda.removeListener(_agendaChanged);
    super.dispose();
  }
}
