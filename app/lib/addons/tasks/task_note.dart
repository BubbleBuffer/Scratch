import 'dart:async';
import 'dart:convert';

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../registry/note_type_registry.dart';
import '../../services/addon_services.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'item_card.dart';
import 'task_agent.dart';
import 'task_composer.dart';
import 'task_data.dart';

Widget taskChip(BuildContext context, NoteArgs args) =>
    TaskNote(args: args, presentation: TaskPresentation.chip);
Widget taskCard(BuildContext context, NoteArgs args) =>
    TaskNote(args: args, presentation: TaskPresentation.card);
Widget taskEmbed(BuildContext context, NoteArgs args) =>
    TaskNote(args: args, presentation: TaskPresentation.embed);
Widget taskPage(BuildContext context, NoteArgs args) =>
    TaskNote(args: args, presentation: TaskPresentation.page);

enum TaskPresentation { chip, card, embed, page }

class TaskNote extends StatefulWidget {
  const TaskNote({super.key, required this.args, required this.presentation});
  final NoteArgs args;
  final TaskPresentation presentation;
  @override
  State<TaskNote> createState() => _TaskNoteState();
}

class _TaskNoteState extends State<TaskNote> {
  TaskData? _task;
  AiTask? _run;
  CalendarSettings? _calendars;
  ChangeFeed? _feed;
  String? _error;
  bool _busy = false;
  int _generation = 0;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final feed = const AddonServices().feed(context);
    if (_feed == feed) return;
    _feed?.removeListener(_changed);
    _feed = feed..addListener(_changed);
    unawaited(_load());
  }

  @override
  void didUpdateWidget(TaskNote oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.args.pageId != widget.args.pageId) {
      _task = null;
      unawaited(_load());
    }
  }

  void _changed(ChangeBatch batch) {
    if (_busy) return;
    unawaited(_load());
  }

  Future<void> _load() async {
    if (!mounted) return;
    final generation = ++_generation;
    try {
      final vault = const AddonServices().vault(context);
      final runs = const AddonServices().agentRuns(context);
      final data = await loadTask(vault, widget.args.pageId);
      final calendars = await vault.listCalendars();
      final runId = data.agentRun;
      final run = runId == null ? null : await runs.run(runId);
      if (mounted && generation == _generation) {
        setState(() {
          _task = data;
          _run = run;
          _calendars = calendars;
          _error = null;
        });
      }
    } catch (_) {
      if (mounted && generation == _generation) {
        setState(() => _error = 'Could not load this task. Try again.');
      }
    }
  }

  Future<void> _toggle() async {
    if (_busy || _task == null) return;
    setState(() => _busy = true);
    try {
      final vault = const AddonServices().vault(context);
      await vault.typeDataSet(widget.args.pageId, taskAddonId, [
        'status',
      ], jsonEncode(_task!.done ? 'todo' : 'done'));
      await vault.flush();
      await _load();
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not update this task. Try again.');
      }
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _save(String title, TaskData task, bool openFull) async {
    if (_busy) return;
    _busy = true;
    try {
      final vault = const AddonServices().vault(context);
      final runs = const AddonServices().agentRuns(context);
      final before = _task?.assignee ?? 'me';
      await saveTask(vault, widget.args.pageId, task);
      try {
        await followTaskAssignee(runs, vault, widget.args.pageId,
            from: before, to: task.assignee, title: title.isEmpty ? widget.args.title : title);
      } catch (e) {
        if (mounted) {
          showErrorToast(
              context, task.assignee == 'agent' ? "The agent couldn't start: $e" : "Couldn't stop the agent: $e");
        }
      }
      await _load();
    } finally {
      _busy = false;
    }
  }

  @override
  void dispose() {
    _generation++;
    _feed?.removeListener(_changed);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final args = widget.args;
    final data = _task;
    if (widget.presentation == TaskPresentation.page) {
      return Column(
        children: [
          ConstrainedBox(
            constraints: BoxConstraints(maxHeight: context.mobile ? 280 : 340),
            child: SingleChildScrollView(
              child: Align(
                alignment: Alignment.center,
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 720),
                  child: Padding(
                    padding: const EdgeInsets.fromLTRB(24, 16, 24, 12),
                    child: data != null && _calendars != null
                        ? Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              TaskFields(
                                initial: data,
                                calendars: _calendars!,
                                onSave: _save,
                                agentRun: _run,
                              ),
                              if (_error != null) _status(context),
                            ],
                          )
                        : _status(context),
                  ),
                ),
              ),
            ),
          ),
          const HLine(),
          Expanded(child: args.editor),
        ],
      );
    }
    if (data == null) return _status(context);
    if (widget.presentation == TaskPresentation.chip) {
      return Hover(
        onTap: args.onOpen,
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (data.checkbox)
              StacksCheckbox(
                checked: data.done,
                size: 16,
                onChanged: _busy ? null : (_) => _toggle(),
              )
            else
              Icon20(Ico.todo, size: 16, color: context.colors.iconTertiary),
            const SizedBox(width: 6),
            Flexible(
              child: Text(
                displayTitle(args.title),
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style:
                    ts(
                      context.type.ui,
                      data.done
                          ? context.colors.textTertiary
                          : context.colors.textPrimary,
                    ).copyWith(
                      decoration: data.done ? TextDecoration.lineThrough : null,
                    ),
              ),
            ),
          ],
        ),
      );
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        ShortItemCard(
          item: data.agendaItem(args.pageId, args.title),
          calendars: _calendars,
          checkbox: data.checkbox,
          undated: data.date == null,
          showDate: true,
          onOpen: args.onOpen,
          onToggle: _toggle,
          busy: _busy,
        ),
        if (_error != null) _status(context),
      ],
    );
  }

  Widget _status(BuildContext context) => Padding(
    padding: const EdgeInsets.all(12),
    child: Row(
      children: [
        Expanded(
          child: Text(
            _error ?? 'Loading task…',
            style: ts(context.type.uiSmall, context.colors.textTertiary),
          ),
        ),
        if (_error != null) TextBtn('Try again', onTap: _load),
      ],
    ),
  );
}
