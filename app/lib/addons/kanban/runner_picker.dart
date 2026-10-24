// The "Run on" picker. Under 700 px it is a bottom sheet; at 700 px and wider
// it is a popover under the button that opened it. It lists the hub and each
// PC, marks the default, and assigns the card to the runner chosen.

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart'
    show SettingsSection, showSettings;
import '../../ui/ui.dart';
import 'runner_backend.dart';
import 'runner_fake.dart' show lastSeenText;

/// Below this width the picker is a bottom sheet; at or above it, a popover.
const runnerSheetBelow = 700.0;

/// Shows the picker for the task [taskId]. Completes with the assignment
/// made, or null when the picker closes without one.
Future<RunnerAssignment?> showRunnerPicker(
  BuildContext context, {
  required Rect anchor,
  required String taskId,
  required RunnerAssignments assignments,
}) {
  if (MediaQuery.sizeOf(context).width < runnerSheetBelow) {
    return showSheet<RunnerAssignment>(
      context,
      (ctx) => SheetSurface(
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10),
            child: _RunnerPicker(taskId: taskId, assignments: assignments),
          ),
        ],
      ),
    );
  }
  return showPopover<RunnerAssignment>(
    context,
    anchor,
    (ctx) => MenuSurface(
      width: 300,
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 12),
      child: _RunnerPicker(taskId: taskId, assignments: assignments),
    ),
  );
}

/// The line under a runner's name: what it is, and whether it is on.
String runnerSubline(Runner runner, DateTime now) {
  if (runner.kind == RunnerKind.hub) return 'Always on · vault tools only';
  if (runner.online) {
    return runner.isThisDevice ? 'This device · online' : 'Online';
  }
  final seen = runner.lastSeen;
  final since = seen == null ? '' : ' · last seen ${lastSeenText(seen, now)}';
  return 'Offline$since';
}

/// The runner the picker starts on: the default, else the first listed.
String? defaultRunnerId(List<Runner> runners) {
  for (final r in runners) {
    if (r.isDefault) return r.id;
  }
  return runners.isEmpty ? null : runners.first.id;
}

String _reasonOf(Object? error) =>
    error is RunnerUnavailable ? error.reason : 'Runners could not be listed.';

class _RunnerPicker extends StatefulWidget {
  const _RunnerPicker({required this.taskId, required this.assignments});

  final String taskId;
  final RunnerAssignments assignments;

  @override
  State<_RunnerPicker> createState() => _RunnerPickerState();
}

class _RunnerPickerState extends State<_RunnerPicker> {
  late final Future<List<Runner>> _runners = widget.assignments.runners();
  String? _chosen;
  bool _busy = false;
  String? _error;

  Future<void> _assign(String runnerId) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      final assignment = await widget.assignments.assign(
        widget.taskId,
        runnerId,
      );
      if (mounted) Navigator.of(context).pop(assignment);
    } catch (e) {
      if (!mounted) return;
      setState(() {
        _busy = false;
        _error = _reasonOf(e);
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return FutureBuilder<List<Runner>>(
      future: _runners,
      builder: (context, snap) {
        final runners = snap.data ?? const <Runner>[];
        final chosen = _chosen ?? defaultRunnerId(runners);
        final now = DateTime.now();
        return Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(2, 0, 2, 8),
              child: Text(
                'Run on',
                style: ts(const TypeSpec(14, 18, 700), c.textPrimary),
              ),
            ),
            if (snap.hasError)
              _Notice(_reasonOf(snap.error))
            else if (snap.connectionState != ConnectionState.done)
              const SizedBox(height: 56)
            else if (runners.isEmpty)
              const _Notice('No runners are connected.')
            else
              for (final r in runners)
                _RunnerRow(
                  runner: r,
                  subline: runnerSubline(r, now),
                  selected: r.id == chosen,
                  onTap: () => setState(() => _chosen = r.id),
                ),
            if (_error != null) _Notice(_error!),
            Container(
              margin: const EdgeInsets.only(top: 12),
              padding: const EdgeInsets.only(top: 12),
              decoration: BoxDecoration(
                border: Border(top: BorderSide(color: c.border)),
              ),
              child: Row(
                children: [
                  TextBtn(
                    'Change default in Settings',
                    color: c.accentText,
                    onTap: () => unawaited(
                      showSettings(context, section: SettingsSection.ai),
                    ),
                  ),
                  const Spacer(),
                  PrimaryButton(
                    key: const ValueKey('runner-assign'),
                    label: 'Assign',
                    height: 38,
                    onTap: chosen == null || _busy
                        ? () {}
                        : () => _assign(chosen),
                  ),
                ],
              ),
            ),
          ],
        );
      },
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(2, 6, 2, 0),
    child: Text(
      text,
      style: ts(const TypeSpec(12.5, 17, 400), context.colors.textSecondary),
    ),
  );
}

class _RunnerRow extends StatelessWidget {
  const _RunnerRow({
    required this.runner,
    required this.subline,
    required this.selected,
    required this.onTap,
  });

  final Runner runner;
  final String subline;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final r = runner;
    return Semantics(
      button: true,
      selected: selected,
      label: r.name,
      child: GestureDetector(
        key: ValueKey('runner-row-${r.id}'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Container(
          margin: const EdgeInsets.only(bottom: 4),
          padding: const EdgeInsets.all(8),
          decoration: BoxDecoration(
            color: selected ? c.bgHover : null,
            borderRadius: Radii.field,
            border: Border.all(
              color: selected ? c.borderStrong : const Color(0x00000000),
            ),
          ),
          child: Row(
            children: [
              Container(
                width: 30,
                height: 30,
                decoration: BoxDecoration(
                  color: c.bgPrimary,
                  borderRadius: Radii.ctl,
                  border: Border.all(color: c.border),
                ),
                child: Center(
                  child: Icon20(
                    r.kind == RunnerKind.hub ? Ico.folder : Ico.terminal,
                    size: 16,
                    color: c.iconSecondary,
                  ),
                ),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            r.name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: ts(
                              const TypeSpec(14.5, 18, 700),
                              c.textPrimary,
                            ),
                          ),
                        ),
                        if (r.isDefault) ...[
                          const SizedBox(width: 6),
                          const _DefaultBadge(),
                        ],
                      ],
                    ),
                    Text(
                      subline,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: ts(
                        const TypeSpec(12.5, 17, 400),
                        c.textSecondary,
                      ),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              _RadioDot(selected: selected),
            ],
          ),
        ),
      ),
    );
  }
}

class _DefaultBadge extends StatelessWidget {
  const _DefaultBadge();

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      key: const ValueKey('runner-default-badge'),
      padding: const EdgeInsets.fromLTRB(5, 2, 5, 3),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.pill,
      ),
      child: Text(
        'default',
        style: ts(const TypeSpec(10.5, 12, 600), c.textSecondary),
      ),
    );
  }
}

class _RadioDot extends StatelessWidget {
  const _RadioDot({required this.selected});

  final bool selected;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Container(
      width: 16,
      height: 16,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: c.bgPrimary,
        border: Border.all(
          color: selected ? c.textPrimary : c.borderStrong,
          width: 1.75,
        ),
      ),
      child: selected
          ? Center(
              child: Container(
                width: 8,
                height: 8,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  color: c.textPrimary,
                ),
              ),
            )
          : null,
    );
  }
}
