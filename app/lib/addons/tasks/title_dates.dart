import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../services/addon_services.dart';
import '../../state/agenda_store.dart' show months;
import '../../ui/ui.dart';
import 'calendar_screen.dart';
import 'task_data.dart';

/// Dates are distinct calendar days, regardless of how often they are mentioned.
List<DateTime> noteMentionDates(List<FlatBlock> blocks) {
  final dates = <String, DateTime>{};
  for (final block in blocks) {
    for (final mark in block.marks) {
      if (mark.kind != BMarkKind.date ||
          mark.start < 0 ||
          mark.end > block.text.length ||
          mark.start >= mark.end) {
        continue;
      }
      final date = DateTime.tryParse(mark.url);
      if (date != null && taskDateString(date) == mark.url) {
        dates[mark.url] = date;
      }
    }
  }
  return dates.values.toList()..sort();
}

DateTime? primaryNoteDate(List<DateTime> dates, DateTime now) {
  if (dates.isEmpty) return null;
  final sorted = List<DateTime>.of(dates)..sort();
  final today = DateTime(now.year, now.month, now.day);
  return sorted.where((date) => !date.isBefore(today)).firstOrNull ??
      sorted.first;
}

// Stay subscribed even when undated so the first newly added mention appears
// while this note remains open. An undated page occupies no title-area space.
Widget? taskTitleDates(BuildContext context, PageData page) =>
    TaskTitleDates(page: page);

/// Scheduling dates join the host's title area without owning its title editor.
class TaskTitleDates extends StatefulWidget {
  const TaskTitleDates({super.key, required this.page});
  final PageData page;

  @override
  State<TaskTitleDates> createState() => _TaskTitleDatesState();
}

class _TaskTitleDatesState extends State<TaskTitleDates> {
  TaskData? _task;
  late List<DateTime> _dates;
  ChangeFeed? _feed;
  int _generation = 0;
  bool _error = false;

  @override
  void initState() {
    super.initState();
    _dates = noteMentionDates(widget.page.blocks);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final feed = const AddonServices().feed(context);
    if (feed == _feed) return;
    _feed?.removeListener(_changed);
    _feed = feed..addListener(_changed);
    unawaited(_load());
  }

  @override
  void didUpdateWidget(TaskTitleDates oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.page != widget.page) {
      _dates = noteMentionDates(widget.page.blocks);
      if (oldWidget.page.id != widget.page.id) _task = null;
      unawaited(_load());
    }
  }

  void _changed(ChangeBatch batch) => unawaited(_load());

  Future<void> _load() async {
    final generation = ++_generation;
    try {
      final vault = const AddonServices().vault(context);
      final page = await vault.loadPage(widget.page.id);
      final task = page.pageType == taskType
          ? await loadTask(vault, page.id)
          : null;
      if (!mounted || generation != _generation) return;
      setState(() {
        _task = task;
        _dates = noteMentionDates(page.blocks);
        _error = false;
      });
    } catch (_) {
      if (mounted && generation == _generation) setState(() => _error = true);
    }
  }

  @override
  void dispose() {
    _generation++;
    _feed?.removeListener(_changed);
    super.dispose();
  }

  String _label(DateTime date, {bool timed = false}) {
    final day =
        '${date.day} ${months[date.month - 1].substring(0, 3)}'
        '${date.year == clockNow().year ? '' : ' ${date.year}'}';
    return timed
        ? '$day · ${date.hour.toString().padLeft(2, '0')}:${date.minute.toString().padLeft(2, '0')}'
        : day;
  }

  Future<void> _openOthers(BuildContext anchorContext, DateTime primary) async {
    final others = _dates.where((date) => date != primary).toList();
    final chosen = await showPopover<DateTime>(
      context,
      globalRectOf(anchorContext),
      (popoverContext) => ConstrainedBox(
        constraints: const BoxConstraints(maxHeight: 320),
        child: SingleChildScrollView(
          child: MenuList(
            width: 220,
            children: [
              for (final date in others)
                MenuItem(
                  key: ValueKey('title-date-option-${taskDateString(date)}'),
                  icon: Ico.clock,
                  label: _label(date),
                  onTap: () => Navigator.of(popoverContext).pop(date),
                ),
            ],
          ),
        ),
      ),
    );
    if (chosen != null && mounted) {
      unawaited(openCalendar(context, day: chosen));
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_error) return TextBtn('Retry date', onTap: _load);
    final taskPage = widget.page.pageType == taskType;
    final date = taskPage
        ? _task?.dateTime
        : primaryNoteDate(_dates, clockNow());
    if (date == null) return const SizedBox.shrink();
    final label = _label(date, timed: taskPage && (_task?.hasTime ?? false));
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Semantics(
            button: true,
            label: 'Open calendar on ${taskDateString(date)}',
            child: Hover(
              key: const ValueKey('note-title-date'),
              onTap: () => unawaited(openCalendar(context, day: date)),
              borderRadius: BorderRadius.circular(16),
              background: c.bgSecondary,
              border: Border.all(color: c.border),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon20(Ico.clock, size: 16, color: c.iconSecondary),
                    const SizedBox(width: 6),
                    Text(
                      label,
                      style: ts(context.type.uiSmall, c.textSecondary),
                    ),
                  ],
                ),
              ),
            ),
          ),
          if (!taskPage && _dates.length > 1)
            Builder(
              builder: (anchorContext) => TextBtn(
                '+${_dates.length - 1}',
                key: const ValueKey('note-title-more-dates'),
                onTap: () => _openOthers(anchorContext, date),
              ),
            ),
        ],
      ),
    );
  }
}
