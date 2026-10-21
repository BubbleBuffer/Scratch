// The schedule's full page: the title, the status band (state, actions, next
// and last run), then When, Runs on and Step.
// Every control writes through [ScheduleController]; this file only draws.

import 'package:flutter/widgets.dart';

import '../../services/legacy_host.dart' show openPage, pickPage;
import '../../ui/app_name.dart';
import '../../ui/database/widgets.dart' show ChipBtn, MenuField, Pill;
import '../../ui/ui.dart';
import 'backend.dart';
import 'controller.dart';
import 'format.dart';
import 'storage.dart';

/// The full schedule view. [pageId] is the schedule note. Storage and backend
/// come from the nearest [ScheduleScope] and [ScheduleBackendScope].
class SchedulePage extends StatefulWidget {
  const SchedulePage({
    super.key,
    required this.pageId,
    this.title = '',
    this.titleController,
    this.onTitleChanged,
    this.deviceTimeZone = 'UTC',
    this.localDeviceId = '',
    this.now,
  });

  final String pageId;
  final String title;
  final TextEditingController? titleController;
  final ValueChanged<String>? onTitleChanged;

  /// The IANA zone a new trigger uses.
  final String deviceTimeZone;

  /// This device's id, to tell its own runner from another desktop.
  final String localDeviceId;

  /// The clock for "last seen" and the default start. The app uses the wall
  /// clock.
  final DateTime Function()? now;

  @override
  State<SchedulePage> createState() => _SchedulePageState();
}

class _SchedulePageState extends State<SchedulePage> {
  ScheduleController? _c;
  final _time = TextEditingController();
  final _monthDay = TextEditingController();
  final _zone = TextEditingController();
  final _stepTitle = TextEditingController();

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_c != null) return;
    final controller = ScheduleController(
      pageId: widget.pageId,
      storage: ScheduleScope.storageOf(context),
      backend: ScheduleBackendScope.of(context),
      deviceTimeZone: widget.deviceTimeZone,
      localDeviceId: widget.localDeviceId,
      now: widget.now,
    )..addListener(_syncFields);
    _c = controller;
    controller.load();
  }

  @override
  void dispose() {
    _c?.removeListener(_syncFields);
    _c?.dispose();
    _time.dispose();
    _monthDay.dispose();
    _zone.dispose();
    _stepTitle.dispose();
    super.dispose();
  }

  ScheduleDef? _synced;

  /// Keeps the text fields showing the definition after a commit. The
  /// controller replaces its definition object on every commit (and on a
  /// failed save reverts to the previous object), so only those notifications
  /// rewrite the fields. A rejected edit changes nothing and keeps the text.
  void _syncFields() {
    final c = _c;
    if (c == null || !c.loaded || identical(_synced, c.def)) return;
    _synced = c.def;
    final trigger = c.def.trigger;
    _setText(_time, trigger.time);
    _setText(_zone, trigger.tz);
    _setText(_monthDay, '${trigger.rule?.monthDay ?? ''}');
    _setText(_stepTitle, c.def.firstStep?.title ?? '');
  }

  void _setText(TextEditingController field, String value) {
    if (field.text != value) field.text = value;
  }

  @override
  Widget build(BuildContext context) {
    final c = _c!;
    final mobile = context.mobile;
    final colors = context.colors;
    return ColoredBox(
      color: colors.bgPrimary,
      child: ListenableBuilder(
        listenable: c,
        builder: (context, _) {
          if (!c.loaded) return const SizedBox.shrink();
          final gutter = mobile ? Layout.gutter : 48.0;
          return Align(
            alignment: Alignment.topCenter,
            child: SingleChildScrollView(
              padding: EdgeInsets.fromLTRB(gutter, mobile ? 16 : 24, gutter, 40),
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 640),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    _ScheduleTitle(
                      title: widget.title,
                      controller: widget.titleController,
                      onChanged: widget.onTitleChanged,
                    ),
                    const SizedBox(height: 12),
                    _StatusBand(c: c),
                    _Section(
                      label: 'When',
                      child: _WhenSection(
                        c: c,
                        time: _time,
                        zone: _zone,
                        monthDay: _monthDay,
                      ),
                    ),
                    _Section(label: 'Runs on', child: _RunnerSection(c: c)),
                    _Section(
                      label: 'Step',
                      child: _StepSection(c: c, titleField: _stepTitle),
                    ),
                  ],
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}

/// A labelled block of the page, separated from the one above by a seam.
class _Section extends StatelessWidget {
  const _Section({required this.label, required this.child});
  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(top: 20),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          HLine(color: c.thread),
          const SizedBox(height: 14),
          Text(
            label,
            style: ts(const TypeSpec(15, 20, 650), c.textPrimary),
          ),
          const SizedBox(height: 10),
          child,
        ],
      ),
    );
  }
}

class _ScheduleTitle extends StatefulWidget {
  const _ScheduleTitle({required this.title, this.controller, this.onChanged});
  final String title;
  final TextEditingController? controller;
  final ValueChanged<String>? onChanged;

  @override
  State<_ScheduleTitle> createState() => _ScheduleTitleState();
}

class _ScheduleTitleState extends State<_ScheduleTitle> {
  final _focus = FocusNode();

  @override
  void dispose() {
    _focus.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final style = tsDisplay(
      context.mobile
          ? const TypeSpec(28, 34, 700)
          : const TypeSpec(32, 40, 700),
      c.textPrimary,
    );
    final controller = widget.controller;
    if (controller == null) {
      return Text(displayTitle(widget.title), style: style);
    }
    return EditableText(
      key: const ValueKey('schedule-title'),
      controller: controller,
      focusNode: _focus,
      style: style,
      cursorColor: c.textPrimary,
      backgroundCursorColor: c.textTertiary,
      selectionColor: c.textSelection,
      cursorWidth: 1.5,
      maxLines: null,
      selectionControls: noteSelectionControls,
      onChanged: widget.onChanged,
    );
  }
}

// ---- When -----------------------------------------------------------------

const _freqLabels = {
  ScheduleFreq.once: 'Once',
  ScheduleFreq.daily: 'Daily',
  ScheduleFreq.weekly: 'Weekly',
  ScheduleFreq.monthly: 'Monthly',
};

class _WhenSection extends StatelessWidget {
  const _WhenSection({
    required this.c,
    required this.time,
    required this.zone,
    required this.monthDay,
  });
  final ScheduleController c;
  final TextEditingController time, zone, monthDay;

  @override
  Widget build(BuildContext context) {
    final trigger = c.def.trigger;
    final rule = trigger.rule;
    final freq = rule == null || !rule.supported
        ? ScheduleFreq.once
        : rule.freq;
    final weekDays = rule?.byDay ?? const <String>[];
    final colors = context.colors;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Text(
          triggerInWords(trigger),
          key: const ValueKey('schedule-words'),
          style: ts(const TypeSpec(14, 20, 500), colors.textSecondary),
        ),
        const SizedBox(height: 10),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            for (final f in ScheduleFreq.values)
              ChipBtn(
                key: ValueKey('schedule-freq-${f.name}'),
                label: _freqLabels[f]!,
                active: freq == f,
                onTap: () => c.setFreq(f),
              ),
          ],
        ),
        if (rule != null && !rule.supported)
          _ErrorText(
            'This repeat rule is not one the editor can show (${rule.toText()}).',
          ),
        if (freq == ScheduleFreq.weekly) ...[
          const SizedBox(height: 10),
          Wrap(
            spacing: 6,
            runSpacing: 6,
            children: [
              for (var i = 0; i < weekdayCodes.length; i++)
                ChipBtn(
                  key: ValueKey('schedule-day-${weekdayCodes[i]}'),
                  label: weekdayNames[i].substring(0, 3),
                  active: weekDays.contains(weekdayCodes[i]),
                  onTap: () {
                    final days = [...weekDays];
                    final code = weekdayCodes[i];
                    if (days.contains(code)) {
                      days.remove(code);
                    } else {
                      days.add(code);
                    }
                    c.setWeekdays(days);
                  },
                ),
            ],
          ),
        ],
        if (freq == ScheduleFreq.monthly) ...[
          const SizedBox(height: 10),
          _LabelledField(
            label: 'Day of month',
            child: MenuField(
              key: const ValueKey('schedule-monthday'),
              controller: monthDay,
              placeholder: '1 to 31',
              numeric: true,
              onSubmitted: c.setMonthDay,
            ),
          ),
          if (c.monthDayError != null) _ErrorText(c.monthDayError!),
        ],
        const SizedBox(height: 10),
        Row(
          children: [
            Expanded(
              child: _LabelledField(
                label: 'Time',
                child: MenuField(
                  key: const ValueKey('schedule-time'),
                  controller: time,
                  placeholder: 'HH:MM',
                  onChanged: c.setTime,
                  onSubmitted: c.setTime,
                ),
              ),
            ),
            const SizedBox(width: 10),
            Expanded(
              flex: 2,
              child: _LabelledField(
                label: 'Time zone',
                child: MenuField(
                  key: const ValueKey('schedule-zone'),
                  controller: zone,
                  placeholder: 'Europe/Berlin',
                  onSubmitted: c.setTimeZone,
                ),
              ),
            ),
          ],
        ),
        if (c.timeError != null) _ErrorText(c.timeError!),
        const SizedBox(height: 10),
        Text(
          'If missed',
          style: ts(const TypeSpec(13, 18, 600), colors.textSecondary),
        ),
        const SizedBox(height: 6),
        Wrap(
          spacing: 8,
          runSpacing: 8,
          children: [
            ChipBtn(
              key: const ValueKey('schedule-missed-latest'),
              label: 'Run the latest once',
              active: trigger.missed == missedLatest,
              onTap: () => c.setMissed(missedLatest),
            ),
            ChipBtn(
              key: const ValueKey('schedule-missed-skip'),
              label: 'Skip',
              active: trigger.missed == missedSkip,
              onTap: () => c.setMissed(missedSkip),
            ),
          ],
        ),
      ],
    );
  }
}

class _LabelledField extends StatelessWidget {
  const _LabelledField({required this.label, required this.child});
  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      Text(
        label,
        style: ts(const TypeSpec(13, 18, 600), context.colors.textSecondary),
      ),
      const SizedBox(height: 6),
      child,
    ],
  );
}

class _ErrorText extends StatelessWidget {
  const _ErrorText(this.text);
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 6),
    child: Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: context.colors.dangerBg,
        borderRadius: Radii.field,
      ),
      child: Text(
        text,
        style: ts(const TypeSpec(13, 18, 500), context.colors.textPrimary),
      ),
    ),
  );
}

// ---- Runs on --------------------------------------------------------------

class _RunnerSection extends StatelessWidget {
  const _RunnerSection({required this.c});
  final ScheduleController c;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final runner = c.def.runner;
    final nodes = c.pickerNodes;
    if (nodes.isEmpty) {
      return Text(
        'No runner is available. Connect a hub in Sync settings.',
        key: const ValueKey('schedule-no-runner'),
        style: ts(const TypeSpec(14, 20, 450), colors.textSecondary),
      );
    }
    final now = c.now();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final node in nodes)
          _NodeRow(
            key: ValueKey('schedule-node-${node.id}'),
            name: node.name,
            detail: node.id == runner && !node.available
                ? '${node.kindLabel} · runner turned off'
                : '${node.kindLabel} · last seen ${_ago(node.lastSeen, now)}',
            selected: node.id == runner,
            onTap: () => c.setRunner(node.id),
          ),
      ],
    );
  }
}

class _NodeRow extends StatelessWidget {
  const _NodeRow({
    super.key,
    required this.name,
    required this.detail,
    required this.selected,
    required this.onTap,
  });
  final String name, detail;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          decoration: BoxDecoration(
            color: selected ? c.bgSelectedBlock : c.bgSecondary,
            borderRadius: Radii.field,
            border: Border.all(
              color: selected ? c.borderStrong : c.border,
            ),
          ),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      name,
                      style: ts(const TypeSpec(15, 20, 600), c.textPrimary),
                    ),
                    Text(
                      detail,
                      style: ts(const TypeSpec(13, 18, 450), c.textSecondary),
                    ),
                  ],
                ),
              ),
              if (selected)
                Text(
                  'Runner',
                  style: ts(const TypeSpec(13, 18, 600), c.textSecondary),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---- Step -----------------------------------------------------------------

class _StepSection extends StatelessWidget {
  const _StepSection({required this.c, required this.titleField});
  final ScheduleController c;
  final TextEditingController titleField;

  @override
  Widget build(BuildContext context) {
    final steps = c.def.steps;
    final colors = context.colors;
    if (steps.length > 1) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            c.refusal ?? '',
            key: const ValueKey('schedule-step-refusal'),
            style: ts(const TypeSpec(14, 20, 500), colors.textPrimary),
          ),
          const SizedBox(height: 8),
          for (final (i, s) in steps.indexed)
            Text(
              '${i + 1}. ${s.kind} (read-only)',
              style: ts(const TypeSpec(14, 20, 450), colors.textSecondary),
            ),
        ],
      );
    }
    final step = c.def.firstStep;
    String labelOf(String? link, String empty) {
      if (pageIdOf(link) == null) return empty;
      if (!c.hasPageTitle(link)) return 'Page not found';
      return c.titleOf(link) ?? 'Untitled page';
    }

    final preview = c.preview;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        _PickerRow(
          key: const ValueKey('schedule-template'),
          label: 'Template',
          value: labelOf(step?.template, 'Choose a template'),
          onTap: () async {
            final id = await pickPage(context, globalRectOf(context));
            if (id != null) await c.setStepTemplate(id);
          },
        ),
        _PickerRow(
          key: const ValueKey('schedule-parent'),
          label: 'Parent',
          value: labelOf(step?.parent, 'Choose a parent page'),
          onTap: () async {
            final id = await pickPage(context, globalRectOf(context));
            if (id != null) await c.setStepParent(id);
          },
        ),
        const SizedBox(height: 4),
        _LabelledField(
          label: 'Title of the new note',
          child: MenuField(
            key: const ValueKey('schedule-step-title'),
            controller: titleField,
            placeholder: 'Week {{week}} {{year}}',
            onChanged: c.setStepTitle,
          ),
        ),
        if (preview != null && preview.ok && preview.renderedTitle.isNotEmpty)
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: Text(
              'Preview: ${preview.renderedTitle}',
              key: const ValueKey('schedule-preview'),
              style: ts(const TypeSpec(13, 18, 450), colors.textSecondary),
            ),
          ),
        if (preview != null && !preview.ok && preview.reason != null)
          _ErrorText(preview.reason!),
        if (c.actionError != null && c.actionError!.isNotEmpty)
          _ErrorText(c.actionError!),
      ],
    );
  }
}

class _PickerRow extends StatelessWidget {
  const _PickerRow({
    super.key,
    required this.label,
    required this.value,
    required this.onTap,
  });
  final String label, value;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.only(bottom: 6),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          constraints: const BoxConstraints(minHeight: 44),
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          decoration: BoxDecoration(
            color: c.bgSecondary,
            borderRadius: Radii.field,
            border: Border.all(color: c.border),
          ),
          child: Row(
            children: [
              Text(label, style: ts(const TypeSpec(14, 20, 600), c.textSecondary)),
              const SizedBox(width: 12),
              Expanded(
                child: Text(
                  value,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  textAlign: TextAlign.end,
                  style: ts(const TypeSpec(15, 20, 450), c.textPrimary),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

// ---- Status band ----------------------------------------------------------

/// The state of the schedule on its runner, the actions that apply to it, and
/// the next and last run. It sits right under the title.
class _StatusBand extends StatelessWidget {
  const _StatusBand({required this.c});
  final ScheduleController c;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final health = c.health;
    final runnerNode = c.nodeOf(c.def.runner);
    final refusal = c.refusal;
    final runs = c.nextRuns;
    final last = c.lastRun;
    final error = c.actionError;
    // The next run is only a plan while the runner is off, the schedule is
    // paused or it has not been accepted: say so in the label.
    final waiting =
        health == ScheduleHealth.runnerOff ||
        health == ScheduleHealth.paused ||
        health == ScheduleHealth.needsAcceptance;
    final actions = _actionButtons(c, health, compact: !context.mobile);
    return Container(
      key: const ValueKey('schedule-status-band'),
      padding: const EdgeInsets.all(12),
      decoration: BoxDecoration(
        color: colors.bgSecondary,
        borderRadius: Radii.panel,
        border: Border.all(color: colors.border),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Pill(
            _healthChipLabel(health, runnerNode),
            key: ValueKey('schedule-health-${health.name}'),
            status: true,
          ),
          if (refusal != null)
            _BandText(refusal, key: const ValueKey('schedule-refusal')),
          if (health == ScheduleHealth.unreachable && c.hostError != null)
            _BandText(c.hostError!, key: const ValueKey('schedule-host-error')),
          if (health == ScheduleHealth.elsewhere)
            _BandText(
              'Open $appName on ${runnerNode?.name ?? 'that desktop'} to '
              'accept or pause',
              key: const ValueKey('schedule-elsewhere-hint'),
            ),
          if (health == ScheduleHealth.noRunner)
            _BandText(
              'Choose a runner below. Nothing runs until one accepts it.',
              key: const ValueKey('schedule-no-runner-hint'),
            ),
          if (health == ScheduleHealth.needsAcceptance)
            const _BandText(
              'Review the schedule, then accept it. Nothing runs until you do.',
            ),
          if (actions.isNotEmpty) ...[
            const SizedBox(height: 12),
            Align(
              alignment: AlignmentDirectional.centerStart,
              // IntrinsicWidth keeps each button at its own width; the shared
              // secondary button centres its label and would otherwise fill
              // the row, stacking the buttons.
              child: Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  for (final action in actions) IntrinsicWidth(child: action),
                ],
              ),
            ),
          ],
          const SizedBox(height: 12),
          _BandLine(
            label: waiting ? 'Next run (when active)' : 'Next run',
            stacked: waiting,
            child: Text(
              runs.isEmpty ? 'None scheduled' : runTimeLabel(runs.first),
              key: const ValueKey('schedule-next-run'),
              style: ts(const TypeSpec(14, 20, 450), colors.textPrimary),
            ),
          ),
          const SizedBox(height: 6),
          _BandLine(
            label: 'Last run',
            child: last == null
                ? Text(
                    'Not run yet',
                    key: const ValueKey('schedule-last-run'),
                    style: ts(const TypeSpec(14, 20, 450), colors.textPrimary),
                  )
                : _LastRunText(last: last),
          ),
          if (error != null && error.isNotEmpty) _ErrorText(error),
        ],
      ),
    );
  }
}

/// The chip label. A hub that does not answer reads as "Hub not reachable".
String _healthChipLabel(ScheduleHealth h, ScheduleNode? runner) {
  if (h == ScheduleHealth.unreachable && runner?.kind == 'hub') {
    return 'Hub not reachable';
  }
  return scheduleHealthLabel(h);
}

class _BandText extends StatelessWidget {
  const _BandText(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: 8),
    child: Text(
      text,
      style: ts(const TypeSpec(14, 20, 500), context.colors.textPrimary),
    ),
  );
}

class _BandLine extends StatelessWidget {
  const _BandLine({required this.label, required this.child, this.stacked = false});
  final String label;
  final Widget child;

  /// The label sits above the value, for a label too long for the side column.
  final bool stacked;

  @override
  Widget build(BuildContext context) {
    final labelStyle = ts(const TypeSpec(13, 18, 600), context.colors.textSecondary);
    if (stacked) {
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(label, style: labelStyle),
          child,
        ],
      );
    }
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 72,
          child: Text(label, style: labelStyle),
        ),
        Expanded(child: child),
      ],
    );
  }
}

class _LastRunText extends StatelessWidget {
  const _LastRunText({required this.last});
  final ScheduleLastRun last;

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    final when = last.at == null ? '' : ' · ${runTimeLabel(last.at!)}';
    if (last.failed) {
      return Text(
        'Failed$when. ${last.error ?? 'The run did not finish.'}',
        key: const ValueKey('schedule-last-run'),
        style: ts(const TypeSpec(14, 20, 500), colors.textPrimary),
      );
    }
    final page = last.page;
    return Row(
      children: [
        Expanded(
          child: Text(
            '${last.result == 'created' ? 'Created' : last.result}$when',
            key: const ValueKey('schedule-last-run'),
            style: ts(const TypeSpec(14, 20, 450), colors.textPrimary),
          ),
        ),
        if (page != null)
          TextBtn(
            'Open note',
            key: const ValueKey('schedule-open-last'),
            onTap: () => openPage(context, page, ''),
          ),
      ],
    );
  }
}

/// The buttons that apply to the schedule now: Accept while it needs
/// acceptance; Pause or Resume and Run now while it runs or is paused. Desktop
/// buttons are compact and sit side by side; a phone keeps touch-sized ones
/// and may wrap them.
List<Widget> _actionButtons(
  ScheduleController c,
  ScheduleHealth health, {
  required bool compact,
}) {
  if (c.def.runner == null) return const [];
  final height = compact ? 34.0 : 42.0;
  final busy = c.busy;
  final paused = health == ScheduleHealth.paused;
  if (health == ScheduleHealth.needsAcceptance) {
    return [
      PrimaryButton(
        key: const ValueKey('schedule-accept'),
        label: 'Accept',
        height: height,
        onTap: () => c.accept(),
      ),
    ];
  }
  if (health == ScheduleHealth.ok || paused) {
    // The controller ignores a second action while one is running (busy).
    return [
      SecondaryButton(
        key: const ValueKey('schedule-pause'),
        label: paused ? 'Resume' : 'Pause',
        height: height,
        onTap: busy ? null : () => c.setPaused(!paused),
      ),
      SecondaryButton(
        key: const ValueKey('schedule-run-now'),
        label: 'Run now',
        height: height,
        onTap: busy ? null : () => c.runNow(),
      ),
    ];
  }
  return const [];
}

String scheduleHealthLabel(ScheduleHealth h) => switch (h) {
  ScheduleHealth.ok => 'Active',
  ScheduleHealth.needsAcceptance => 'Needs acceptance',
  ScheduleHealth.paused => 'Paused',
  ScheduleHealth.runnerOff => 'Runner turned off',
  ScheduleHealth.refused => 'Refused',
  ScheduleHealth.noRunner => 'No runner',
  ScheduleHealth.unreachable => 'Not reachable',
  ScheduleHealth.elsewhere => 'Another desktop',
};

const _months = [
  'Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun',
  'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec',
];

/// "Mon 19 Oct, 08:00" for a local instant.
String runTimeLabel(DateTime d) {
  final hh = d.hour.toString().padLeft(2, '0');
  final mm = d.minute.toString().padLeft(2, '0');
  return '${weekdayNames[d.weekday - 1].substring(0, 3)} ${d.day} '
      '${_months[d.month - 1]}, $hh:$mm';
}

String _ago(DateTime? seen, DateTime now) {
  if (seen == null) return 'a while ago';
  final d = now.difference(seen);
  if (d.inMinutes < 1) return 'just now';
  if (d.inMinutes < 60) return '${d.inMinutes} min ago';
  if (d.inHours < 24) return '${d.inHours} h ago';
  return '${d.inDays} days ago';
}
