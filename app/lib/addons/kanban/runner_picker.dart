// The "Run on" picker. Under 700 px it is a bottom sheet; at 700 px and wider
// it is a popover under the button that opened it. It lists the hub and each
// PC, marks the default, and assigns the card to the runner chosen. Under a
// PC that has projects it also offers a Project choice, None by default.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../services/addon_services.dart'
    show SettingsSection, showSettings;
import '../../ui/ui.dart';
import 'keyboard_menu.dart';
import 'runner_backend.dart';

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
            padding: const EdgeInsets.symmetric(horizontal: Space.md),
            child: _RunnerPicker(taskId: taskId, assignments: assignments),
          ),
        ],
      ),
    );
  }
  return Navigator.of(context).push(
    _PickerRoute(
      anchor: anchor,
      duration: Motion.of(context).fast,
      builder: (ctx) => MenuSurface(
        width: BoardSize.runnerPicker,
        padding: const EdgeInsets.fromLTRB(Space.lg, Space.lg, Space.lg, Space.md),
        child: _RunnerPicker(taskId: taskId, assignments: assignments),
      ),
    ),
  );
}

/// The picker's popover. It always opens below [anchor], so it never covers
/// the column headers above the card. The kit's popover flips above the
/// anchor when there is no room below; this one moves up only as far as the
/// window needs.
class _PickerRoute extends PopupRoute<RunnerAssignment> {
  _PickerRoute({
    required this.anchor,
    required this.builder,
    required this.duration,
  });

  final Rect anchor;
  final WidgetBuilder builder;
  final Duration duration;

  @override
  Color? get barrierColor => null;
  @override
  bool get barrierDismissible => true;
  @override
  String? get barrierLabel => 'Close';
  @override
  Duration get transitionDuration => duration;

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) => CustomSingleChildLayout(
    delegate: _PickerLayout(anchor),
    child: builder(context),
  );

  @override
  Widget buildTransitions(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) => MediaQuery.disableAnimationsOf(context)
      ? child
      : FadeTransition(opacity: animation, child: child);
}

class _PickerLayout extends SingleChildLayoutDelegate {
  _PickerLayout(this.anchor);

  final Rect anchor;

  static const _margin = 8.0;

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) =>
      constraints.loosen();

  @override
  Offset getPositionForChild(Size size, Size child) {
    // Right-aligned with the button, like a menu from a card's corner, kept
    // inside the window.
    final x = math.max(
      _margin,
      math.min(anchor.right - child.width, size.width - child.width - _margin),
    );
    final y = math.max(
      _margin,
      math.min(anchor.bottom + 4, size.height - child.height - _margin),
    );
    return Offset(x, y);
  }

  @override
  bool shouldRelayout(_PickerLayout old) => old.anchor != anchor;
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

  /// The project of the chosen PC, or null for None. Switching runners resets
  /// it, so it always belongs to the runner chosen.
  String? _project;
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
        projectId: _project,
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
        return KeyboardMenu(
          focusFirst: false,
          child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(Space.hair, 0, Space.hair, Space.sm),
              child: Text(
                'Run on',
                style: ts(context.type.ui, c.textPrimary, weight: Weight.bold),
              ),
            ),
            if (snap.hasError)
              _Notice(_reasonOf(snap.error))
            else if (snap.connectionState != ConnectionState.done)
              const Gap(Space.xxxl)
            else if (runners.isEmpty)
              const _Notice('No runners are connected.')
            else
              for (final r in runners) ...[
                _RunnerRow(
                  runner: r,
                  subline: runnerSubline(r, now),
                  selected: r.id == chosen,
                  onTap: () => setState(() {
                    if (r.id != chosen) _project = null;
                    _chosen = r.id;
                  }),
                ),
                if (r.id == chosen && r.projects.isNotEmpty)
                  _ProjectChoice(
                    projects: r.projects,
                    selected: _project,
                    onTap: (id) => setState(() => _project = id),
                  ),
              ],
            if (_error != null) _Notice(_error!),
            Container(
              margin: const EdgeInsets.only(top: Space.md),
              padding: const EdgeInsets.only(top: Space.md),
              decoration: BoxDecoration(
                border: Border(top: BorderSide(color: c.border)),
              ),
              child: Row(
                children: [
                  TextBtn(
                    'Change default in Settings',
                    color: c.accentText,
                    onTap: () => unawaited(
                      showSettings(context, section: SettingsSection.agent),
                    ),
                  ),
                  const Spacer(),
                  PrimaryButton(
                    key: const ValueKey('runner-assign'),
                    label: 'Assign',
                    size: ControlSize.lg,
                    onTap: chosen == null || _busy
                        ? () {}
                        : () => _assign(chosen),
                  ),
                ],
              ),
            ),
          ],
          ),
        );
      },
    );
  }
}

/// The Project block under the chosen PC: None, then each project by name.
class _ProjectChoice extends StatelessWidget {
  const _ProjectChoice({
    required this.projects,
    required this.selected,
    required this.onTap,
  });

  final List<RunnerProject> projects;

  /// The chosen project's id, or null for None.
  final String? selected;

  /// Called with the project's id, or null for None.
  final ValueChanged<String?> onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(Space.xxxl, 0, 0, Space.xxs),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        mainAxisSize: MainAxisSize.min,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(0, Space.hair, 0, Space.xxs),
            child: Text(
              'Project',
              style: ts(context.type.uiSmall, c.textSecondary, weight: Weight.semibold),
            ),
          ),
          _ProjectRow(
            key: const ValueKey('project-row-none'),
            name: 'None',
            selected: selected == null,
            onTap: () => onTap(null),
          ),
          for (final p in projects)
            _ProjectRow(
              key: ValueKey('project-row-${p.id}'),
              name: p.name,
              selected: selected == p.id,
              onTap: () => onTap(p.id),
            ),
        ],
      ),
    );
  }
}

/// One project choice: the name and a radio dot, styled like a runner row.
class _ProjectRow extends StatelessWidget {
  const _ProjectRow({
    super.key,
    required this.name,
    required this.selected,
    required this.onTap,
  });

  final String name;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      selected: selected,
      label: name,
      child: _FocusRow(
        onTap: onTap,
        child: Container(
          margin: const EdgeInsets.only(bottom: Space.xxs),
          padding: const EdgeInsets.fromLTRB(Space.sm, Space.xs, Space.sm, Space.xs),
          decoration: BoxDecoration(
            color: selected ? c.bgHover : null,
            borderRadius: Radii.field,
            border: Border.all(
              color: selected ? c.borderStrong : kTransparent,
            ),
          ),
          child: Row(
            children: [
              Expanded(
                child: Text(
                  name,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(context.type.ui, c.textPrimary, weight: Weight.regular),
                ),
              ),
              const Gap(Space.sm),
              _RadioDot(selected: selected),
            ],
          ),
        ),
      ),
    );
  }
}

class _Notice extends StatelessWidget {
  const _Notice(this.text);

  final String text;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(Space.hair, Space.xs, Space.hair, 0),
    child: Text(
      text,
      style: ts(context.type.uiSmall, context.colors.textSecondary, weight: Weight.regular),
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
      child: _FocusRow(
        key: ValueKey('runner-row-${r.id}'),
        autofocus: selected,
        onTap: onTap,
        child: Container(
          margin: const EdgeInsets.only(bottom: Space.xxs),
          padding: const EdgeInsets.all(Space.sm),
          decoration: BoxDecoration(
            color: selected ? c.bgHover : null,
            borderRadius: Radii.field,
            border: Border.all(
              color: selected ? c.borderStrong : kTransparent,
            ),
          ),
          child: Row(
            children: [
              Container(
                width: BoardSize.runnerTile,
                height: BoardSize.runnerTile,
                decoration: BoxDecoration(
                  color: c.bgPrimary,
                  borderRadius: Radii.ctl,
                  border: Border.all(color: c.border),
                ),
                child: Center(
                  child: Icon20(
                    r.kind == RunnerKind.hub ? Ico.folder : Ico.terminal,
                    size: IconSize.sm,
                    color: c.iconSecondary,
                  ),
                ),
              ),
              const Gap(Space.md),
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
                              context.type.ui,
                              c.textPrimary,
                             weight: Weight.bold),
                          ),
                        ),
                        if (r.isDefault) ...[
                          const Gap(Space.xs),
                          const _DefaultBadge(),
                        ],
                      ],
                    ),
                    Text(
                      subline,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: ts(
                        context.type.uiSmall,
                        c.textSecondary,
                       weight: Weight.regular),
                    ),
                  ],
                ),
              ),
              const Gap(Space.sm),
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
      padding: const EdgeInsets.fromLTRB(Space.xs, Space.hair, Space.xs, Space.xxs),
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: Radii.pill,
      ),
      child: Text(
        'default',
        style: ts(context.type.micro, c.textSecondary),
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
      width: SettingsSize.radio,
      height: SettingsSize.radio,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        color: c.bgPrimary,
        border: Border.all(
          color: selected ? c.textPrimary : c.borderStrong,
          width: Stroke.bold,
        ),
      ),
      child: selected
          ? Center(
              child: Container(
                width: Sizes.dot,
                height: Sizes.dot,
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

/// A picker row that takes keyboard focus: Space or Enter selects it, and a
/// ring shows while it has focus from the keyboard. [autofocus] puts focus
/// on it when the picker opens.
class _FocusRow extends StatefulWidget {
  const _FocusRow({
    super.key,
    required this.onTap,
    required this.child,
    this.autofocus = false,
  });

  final VoidCallback onTap;
  final Widget child;
  final bool autofocus;

  @override
  State<_FocusRow> createState() => _FocusRowState();
}

class _FocusRowState extends State<_FocusRow> {
  bool _highlight = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return FocusableActionDetector(
      autofocus: widget.autofocus,
      onShowFocusHighlight: (v) => setState(() => _highlight = v),
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
        SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
      },
      actions: {
        ActivateIntent: CallbackAction<ActivateIntent>(
          onInvoke: (_) {
            widget.onTap();
            return null;
          },
        ),
      },
      child: DecoratedBox(
        position: DecorationPosition.foreground,
        decoration: BoxDecoration(
          borderRadius: Radii.field,
          border: Border.all(
            color: _highlight ? c.focus : kTransparent,
            width: Stroke.bold,
          ),
        ),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: widget.child,
        ),
      ),
    );
  }
}
