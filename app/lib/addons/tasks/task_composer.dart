import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../core/debounce.dart';
import '../../services/addon_services.dart';
import '../../ui/toast.dart';
import '../../ui/ui.dart';
import 'task_agent.dart';
import 'task_data.dart';

class TaskComposer extends StatefulWidget {
  const TaskComposer({
    super.key,
    required this.initialDate,
    this.initialCalendar,
    this.initialHour,
    this.onCreated,
    this.onOpenFull,
    this.fromToday = false,
  });
  final DateTime initialDate;
  final String? initialCalendar;
  final int? initialHour;
  final ValueChanged<String>? onCreated, onOpenFull;
  final bool fromToday;
  @override
  State<TaskComposer> createState() => _TaskComposerState();
}

class _TaskComposerState extends State<TaskComposer> {
  CalendarSettings? _calendars;
  bool _started = false;
  String? _error, _createdPage;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_started) return;
    _started = true;
    unawaited(_load());
  }

  Future<void> _load() async {
    try {
      final calendars = await const AddonServices()
          .vault(context)
          .listCalendars();
      if (mounted) {
        setState(() {
          _calendars = calendars;
          _error = null;
        });
      }
    } catch (_) {
      if (mounted) {
        setState(() => _error = 'Could not load calendars. Try again.');
      }
    }
  }

  Future<void> _create(String title, TaskData task, bool openFull) async {
    final vault = const AddonServices().vault(context);
    final runs = const AddonServices().agentRuns(context);
    // A metadata-write retry completes this page rather than creating a duplicate.
    _createdPage ??= await vault.createPage(title: title, pageType: taskType);
    await vault.renamePage(_createdPage!, title);
    await saveTask(vault, _createdPage!, task);
    if (task.assignee == 'agent') {
      try {
        await followTaskAssignee(runs, vault, _createdPage!, from: 'me', to: 'agent', title: title);
      } catch (e) {
        if (mounted) showErrorToast(context, "The agent couldn't start: $e");
      }
    }
    if (!mounted) return;
    if (openFull) {
      (widget.onOpenFull ?? widget.onCreated)?.call(_createdPage!);
    } else {
      widget.onCreated?.call(_createdPage!);
    }
  }

  @override
  Widget build(BuildContext context) {
    final initial = widget.initialHour == null
        ? widget.initialDate
        : DateTime(
            widget.initialDate.year,
            widget.initialDate.month,
            widget.initialDate.day,
            widget.initialHour!,
          );
    return Padding(
      padding: const EdgeInsets.all(20),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  'New task',
                  style: tsDisplay(context.type.h3, context.colors.textPrimary),
                ),
              ),
              IconBtn(
                Ico.close,
                semanticLabel: 'Close new task',
                onTap: () => Navigator.of(context).maybePop(),
              ),
            ],
          ),
          const SizedBox(height: 16),
          if (_calendars == null) ...[
            Text(
              _error ?? 'Loading calendars…',
              style: ts(context.type.ui, context.colors.textSecondary),
            ),
            if (_error != null) TextBtn('Try again', onTap: _load),
          ] else
            TaskFields(
              initial: TaskData(
                date: taskDateString(
                  initial,
                  timed: widget.initialHour != null,
                ),
                durationMin: widget.initialHour == null ? null : 60,
                calendar:
                    widget.initialCalendar ??
                    (_calendars!.calendars.isEmpty
                        ? null
                        : _calendars!.calendars.first.id),
              ),
              calendars: _calendars!,
              composer: true,
              fromToday: widget.fromToday,
              onSave: _create,
            ),
        ],
      ),
    );
  }
}

/// Scheduling fields shared by the composer and the task's full page.
class TaskFields extends StatefulWidget {
  const TaskFields({
    super.key,
    required this.initial,
    required this.calendars,
    required this.onSave,
    this.composer = false,
    this.fromToday = false,
    this.agentRun,
  });
  final TaskData initial;
  final CalendarSettings calendars;

  /// The agent's run for this task, for its status line.
  final AiTask? agentRun;
  final Future<void> Function(String title, TaskData task, bool openFull)
  onSave;
  final bool composer, fromToday;
  @override
  State<TaskFields> createState() => _TaskFieldsState();
}

class _TaskFieldsState extends State<TaskFields> {
  final _title = TextEditingController();
  final _date = TextEditingController();
  final _time = TextEditingController();
  final _duration = TextEditingController();
  String _status = 'todo', _assignee = 'me';
  String? _calendar, _error;
  bool _timed = false, _checkbox = true, _busy = false, _dirty = false;
  Timer? _autoSave;
  int _editGeneration = 0;

  @override
  void initState() {
    super.initState();
    _adopt();
  }

  void _adopt() {
    final data = widget.initial;
    final date = data.dateTime;
    _date.text = date == null ? '' : taskDateString(date);
    _time.text = date == null || !data.hasTime
        ? '09:00'
        : '${date.hour.toString().padLeft(2, '0')}:${date.minute.toString().padLeft(2, '0')}';
    _duration.text = '${data.durationMin ?? 60}';
    _timed = data.hasTime;
    _status = data.status;
    _assignee = data.assignee;
    _checkbox = data.checkbox;
    _calendar = data.calendar;
  }

  @override
  void didUpdateWidget(TaskFields oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!_dirty && !_busy && oldWidget.initial != widget.initial) _adopt();
  }

  @override
  void dispose() {
    _autoSave?.cancel();
    _title.dispose();
    _date.dispose();
    _time.dispose();
    _duration.dispose();
    super.dispose();
  }

  DateTime? _validDate(String value) {
    if (!RegExp(r'^\d{4}-\d{2}-\d{2}$').hasMatch(value)) return null;
    final date = DateTime.tryParse(value);
    return date != null && taskDateString(date) == value ? date : null;
  }

  Future<void> _save({bool openFull = false}) async {
    if (_busy) return;
    _autoSave?.cancel();
    final editGeneration = _editGeneration;
    final title = _title.text.trim();
    if (widget.composer && title.isEmpty) {
      setState(() => _error = 'Give the task a title.');
      return;
    }
    final rawDate = _date.text.trim();
    var date = _validDate(rawDate);
    if ((widget.composer || rawDate.isNotEmpty) && date == null) {
      setState(() => _error = 'Choose a valid date (YYYY-MM-DD).');
      return;
    }
    int? duration;
    if (_timed) {
      final match = RegExp(r'^(\d{2}):(\d{2})$').firstMatch(_time.text.trim());
      final hour = match == null ? -1 : int.parse(match.group(1)!);
      final minute = match == null ? -1 : int.parse(match.group(2)!);
      duration = int.tryParse(_duration.text.trim());
      if (date == null || hour < 0 || hour > 23 || minute < 0 || minute > 59) {
        setState(() => _error = 'Choose a date and a valid time (HH:MM).');
        return;
      }
      if (duration == null || duration < 0 || duration > 10080) {
        setState(() => _error = 'Use a duration between 0 and 10080 minutes.');
        return;
      }
      date = DateTime(date.year, date.month, date.day, hour, minute);
    }
    // Deleting a category retains the task's id; its other fields stay editable.
    if (_calendar != null &&
        _calendar != widget.initial.calendar &&
        !widget.calendars.calendars.any((c) => c.id == _calendar)) {
      setState(() => _error = 'Choose an available calendar.');
      return;
    }
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await widget.onSave(
        title,
        TaskData(
          status: _status,
          checkbox: _checkbox,
          date: date == null ? null : taskDateString(date, timed: _timed),
          durationMin: duration,
          assignee: _assignee,
          calendar: _calendar,
        ),
        openFull,
      );
      if (mounted) {
        setState(() {
          _dirty = editGeneration != _editGeneration;
          _error = null;
        });
      }
    } catch (_) {
      if (mounted) {
        setState(
          () => _error =
              'Could not save the task. Your draft is still here. Try again.',
        );
      }
    } finally {
      if (mounted) {
        setState(() => _busy = false);
        if (_dirty && _error == null && !widget.composer) _scheduleSave();
      }
    }
  }

  void _scheduleSave() {
    if (widget.composer) return;
    _autoSave?.cancel();
    _autoSave = Timer(
      Debounce.testDelay(const Duration(milliseconds: 180)),
      () {
        if (mounted && _dirty && !_busy) unawaited(_save());
      },
    );
  }

  void _change(VoidCallback update) {
    setState(() {
      update();
      _dirty = true;
      _editGeneration++;
    });
    _scheduleSave();
  }

  Widget _label(String name, Widget field) => Padding(
    padding: const EdgeInsets.only(bottom: 12),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          name,
          style: ts(context.type.uiSmall, context.colors.textTertiary),
        ),
        const SizedBox(height: 4),
        field,
      ],
    ),
  );
  Widget _field(
    String label,
    TextEditingController controller, {
    String placeholder = '',
    bool autofocus = false,
  }) => Focus(
    onFocusChange: (focused) {
      if (!focused && _dirty) _scheduleSave();
    },
    child: Semantics(
      label: label,
      child: SettingsField(
        controller: controller,
        placeholder: placeholder,
        fieldKey: ValueKey('task-${label.toLowerCase().replaceAll(' ', '-')}'),
        autofocus: autofocus,
        onChanged: (_) => setState(() {
          _dirty = true;
          _editGeneration++;
        }),
        onSubmitted: (_) => _save(),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    CalendarCategory? selected;
    for (final category in widget.calendars.calendars) {
      if (category.id == _calendar) selected = category;
    }
    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.enter, control: true): () =>
            _save(),
      },
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (widget.composer)
            _label(
              'Task title',
              _field(
                'Title',
                _title,
                placeholder: 'What needs doing?',
                autofocus: true,
              ),
            ),
          if (!widget.composer)
            Row(
              children: [
                if (_checkbox) ...[
                  StacksCheckbox(
                    checked: _status == 'done',
                    onChanged: _busy
                        ? null
                        : (done) =>
                              _change(() => _status = done ? 'done' : 'todo'),
                  ),
                  const SizedBox(width: 8),
                ],
                SelectButton<String>(
                  value: _status,
                  options: taskStatuses,
                  title: 'Status',
                  labelOf: (value) => switch (value) {
                    'doing' => 'In progress',
                    'done' => 'Done',
                    _ => 'To do',
                  },
                  onChanged: _busy
                      ? null
                      : (value) => _change(() => _status = value),
                ),
                const Spacer(),
                TextBtn(
                  _checkbox ? 'Hide checkbox' : 'Show checkbox',
                  onTap: _busy
                      ? null
                      : () => _change(() => _checkbox = !_checkbox),
                ),
              ],
            ),
          _label(
            'Date',
            Row(
              children: [
                Expanded(
                  child: _field('Date', _date, placeholder: 'YYYY-MM-DD'),
                ),
                if (!widget.composer)
                  TextBtn(
                    'Clear',
                    onTap: _busy
                        ? null
                        : () => _change(() {
                            _date.clear();
                            _timed = false;
                          }),
                  ),
              ],
            ),
          ),
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Row(
              children: [
                StacksCheckbox(
                  checked: _timed,
                  onChanged: _busy
                      ? null
                      : (value) => _change(() => _timed = value),
                ),
                const SizedBox(width: 8),
                Text('Add time', style: ts(context.type.ui, c.textSecondary)),
              ],
            ),
          ),
          if (_timed)
            Row(
              children: [
                Expanded(
                  child: _label(
                    'Time',
                    _field('Time', _time, placeholder: '09:00'),
                  ),
                ),
                const SizedBox(width: 12),
                Expanded(
                  child: _label(
                    'Duration (min)',
                    _field('Duration', _duration, placeholder: '60'),
                  ),
                ),
              ],
            ),
          _label(
            'Calendar',
            SelectButton<String>(
              value: _calendar ?? '',
              options: ['', ...widget.calendars.calendars.map((c) => c.id)],
              title: 'Calendar',
              labelOf: (id) {
                for (final c in widget.calendars.calendars) {
                  if (c.id == id) return '● ${c.name}';
                }
                return id.isEmpty ? 'No calendar' : 'Unavailable calendar';
              },
              onChanged: _busy
                  ? null
                  : (id) => _change(() => _calendar = id.isEmpty ? null : id),
            ),
          ),
          if (widget.fromToday && selected?.showInToday == false)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(
                'This calendar is hidden from Today. Your task will appear in Calendar.',
                style: ts(context.type.uiSmall, c.textTertiary),
              ),
            ),
          _label(
            'Assignee',
            SelectButton<String>(
              value: _assignee,
              options: const ['me', 'agent'],
              title: 'Assignee',
              labelOf: (value) => value == 'agent' ? 'Agent' : 'Me',
              onChanged: _busy
                  ? null
                  : (value) => _change(() => _assignee = value),
            ),
          ),
          if (_assignee == 'agent' || widget.initial.assignee == 'agent')
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Text(
                taskAgentNote(
                  composer: widget.composer,
                  saved: widget.initial,
                  assignee: _assignee,
                  run: widget.agentRun,
                ),
                key: const ValueKey('task-agent-note'),
                style: ts(context.type.uiSmall, c.textTertiary),
              ),
            ),
          if (_error != null)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Semantics(
                liveRegion: true,
                child: Text(
                  _error!,
                  style: ts(context.type.uiSmall, c.inlineCodeText),
                ),
              ),
            ),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              if (widget.composer) ...[
                TextBtn(
                  'Open full',
                  onTap: _busy ? null : () => _save(openFull: true),
                ),
                const SizedBox(width: 12),
              ],
              DialogButton(
                label: _busy
                    ? 'Saving…'
                    : widget.composer
                    ? 'Create task'
                    : 'Save task',
                onTap: _busy || (!widget.composer && !_dirty)
                    ? null
                    : () => _save(),
                filled: true,
              ),
            ],
          ),
        ],
      ),
    );
  }
}
