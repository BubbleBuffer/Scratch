// The panel a board card opens: its title, status (the column it is in), the
// runner chip, an Assign to Agent button that opens the Run on picker, and a
// link to the full note. On desktop it sits beside the board; on a phone it
// is a bottom sheet. Esc closes it.

import 'dart:async';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../../ui/ui.dart';
import 'runner_backend.dart';
import 'runner_chip.dart';

class KanbanCardPanel extends StatefulWidget {
  const KanbanCardPanel({
    super.key,
    required this.cardId,
    required this.title,
    required this.status,
    required this.firstLine,
    required this.runners,
    required this.onClose,
    required this.onOpenNote,
    required this.onAssign,
    this.onStatus,
    this.onAccept,
    this.onMarkFailed,
    this.onAddStep,
    this.onReply,
    this.onOpenSession,
    this.sheet = false,
  });

  final String cardId;
  final String title;

  /// The title of the column the card is in.
  final String status;
  final String firstLine;

  /// Null when the board offers no runners; the panel then has no runner row
  /// and no Assign button.
  final RunnerAssignments? runners;
  final VoidCallback onClose;
  final VoidCallback onOpenNote;

  /// Opens the Run on picker under [anchor], the Assign button.
  final void Function(Rect anchor) onAssign;

  /// Opens the choice of column under [anchor], the Status row. Null: the
  /// row only shows the status.
  final void Function(Rect anchor)? onStatus;

  /// Accepts the run's result; the card moves to the board's last column.
  final VoidCallback? onAccept;

  /// Fails a stalled run.
  final VoidCallback? onMarkFailed;

  /// Adds a step to the running task.
  final void Function(String text)? onAddStep;

  /// Answers the worker's open question.
  final void Function(String text)? onReply;

  /// Opens the chat of the run's session.
  final VoidCallback? onOpenSession;

  /// True in a bottom sheet, which has its own surface and no close button.
  final bool sheet;

  @override
  State<KanbanCardPanel> createState() => _KanbanCardPanelState();
}

class _KanbanCardPanelState extends State<KanbanCardPanel> {
  final _scope = FocusScopeNode(debugLabel: 'kanban card panel');

  /// Focus rests on the title when the panel opens, so no control shows a
  /// focus ring until the user tabs. Tab skips it.
  final _titleFocus = FocusNode(
    debugLabel: 'kanban card panel title',
    skipTraversal: true,
  );
  final _assignKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    // Tab skips the title, but Tab pressed while it has focus moves on.
    // Set after the focus manager finishes its pass, not inside it.
    _titleFocus.addListener(
      () => scheduleMicrotask(() {
        if (mounted) _titleFocus.skipTraversal = !_titleFocus.hasPrimaryFocus;
      }),
    );
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _titleFocus.requestFocus();
    });
  }

  @override
  void dispose() {
    _titleFocus.dispose();
    _scope.dispose();
    super.dispose();
  }

  void _assign() {
    final box = _assignKey.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return;
    widget.onAssign(box.localToGlobal(Offset.zero) & box.size);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final runners = widget.runners;
    return FocusScope(
      node: _scope,
      child: Shortcuts(
        shortcuts: const {
          SingleActivator(LogicalKeyboardKey.escape): DismissIntent(),
        },
        child: Actions(
          actions: {
            DismissIntent: CallbackAction<DismissIntent>(
              onInvoke: (_) {
                widget.onClose();
                return null;
              },
            ),
          },
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 16, 20, 20),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                if (!widget.sheet)
                  Align(
                    alignment: Alignment.centerRight,
                    child: IconBtn(
                      Ico.close,
                      key: const ValueKey('kanban-panel-close'),
                      semanticLabel: 'Close',
                      onTap: widget.onClose,
                    ),
                  ),
                Focus(
                  focusNode: _titleFocus,
                  child: Text(
                    widget.title,
                    key: const ValueKey('kanban-panel-title'),
                    style: tsDisplay(const TypeSpec(22, 28, 700), c.textPrimary),
                  ),
                ),
                if (widget.firstLine.trim().isNotEmpty)
                  Padding(
                    padding: const EdgeInsets.only(top: 6),
                    child: Text(
                      widget.firstLine.trim(),
                      maxLines: 3,
                      overflow: TextOverflow.ellipsis,
                      style: ts(const TypeSpec(14, 20, 400), c.textSecondary),
                    ),
                  ),
                const SizedBox(height: 16),
                _Prop(
                  label: 'Status',
                  child: _ChoiceRow(
                    key: const ValueKey('kanban-panel-status-row'),
                    label: 'Status',
                    value: widget.status,
                    onTap: widget.onStatus,
                    child: Text(
                      widget.status,
                      key: const ValueKey('kanban-panel-status'),
                      style: ts(const TypeSpec(14, 20, 400), c.textPrimary),
                    ),
                  ),
                ),
                if (runners != null)
                  _Prop(
                    label: 'Runner',
                    child: ListenableBuilder(
                      listenable: runners,
                      builder: (context, _) {
                        final a = runners.of(widget.cardId);
                        return _ChoiceRow(
                          key: const ValueKey('kanban-panel-runner-row'),
                          label: 'Runner',
                          value: a == null ? 'Not assigned' : runnerChipText(a),
                          onTap: widget.onAssign,
                          child: a == null
                              ? Text(
                                  'Not assigned',
                                  style: ts(
                                    const TypeSpec(14, 20, 400),
                                    c.textSecondary,
                                  ),
                                )
                              : RunnerChip(assignment: a),
                        );
                      },
                    ),
                  ),
                const SizedBox(height: 16),
                if (runners != null)
                  ListenableBuilder(
                    listenable: runners,
                    builder: (context, _) => runners.of(widget.cardId) != null
                        ? const SizedBox.shrink()
                        : KeyedSubtree(
                            key: _assignKey,
                            child: PrimaryButton(
                              key: const ValueKey('kanban-panel-assign'),
                              label: 'Assign to Agent',
                              icon: Ico.sendHorizontal,
                              onTap: _assign,
                            ),
                          ),
                  ),
                if (runners != null)
                  ListenableBuilder(
                    listenable: runners,
                    builder: (context, _) => _RunActions(
                      state: runners.of(widget.cardId)?.state,
                      question: runners.of(widget.cardId)?.question,
                      hasSession: runners.sessionOf(widget.cardId) != null,
                      onAccept: widget.onAccept,
                      onMarkFailed: widget.onMarkFailed,
                      onAddStep: widget.onAddStep,
                      onReply: widget.onReply,
                      onOpenSession: widget.onOpenSession,
                    ),
                  ),
                const SizedBox(height: 8),
                SecondaryButton(
                  key: const ValueKey('kanban-panel-open-note'),
                  label: 'Open note',
                  onTap: widget.onOpenNote,
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// The run's actions under the runner row: Accept on a result waiting for
/// review, Mark failed on a stalled run, Add step on a running one, and Open
/// session whenever the run has a chat.
class _RunActions extends StatefulWidget {
  const _RunActions({
    required this.state,
    required this.question,
    required this.hasSession,
    required this.onAccept,
    required this.onMarkFailed,
    required this.onAddStep,
    required this.onReply,
    required this.onOpenSession,
  });

  final AssignState? state;
  final String? question;
  final bool hasSession;
  final VoidCallback? onAccept;
  final VoidCallback? onMarkFailed;
  final void Function(String text)? onAddStep;
  final void Function(String text)? onReply;
  final VoidCallback? onOpenSession;

  @override
  State<_RunActions> createState() => _RunActionsState();
}

class _RunActionsState extends State<_RunActions> {
  final _step = TextEditingController();
  final _stepFocus = FocusNode();
  final _reply = TextEditingController();
  final _replyFocus = FocusNode();

  @override
  void dispose() {
    _step.dispose();
    _stepFocus.dispose();
    _reply.dispose();
    _replyFocus.dispose();
    super.dispose();
  }

  void _add(String value) {
    final text = value.trim();
    final add = widget.onAddStep;
    if (text.isEmpty || add == null) return;
    _step.clear();
    add(text);
  }

  void _answer(String value) {
    final text = value.trim();
    final reply = widget.onReply;
    if (text.isEmpty || reply == null) return;
    _reply.clear();
    reply(text);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final state = widget.state;
    final children = <Widget>[];
    if (state == AssignState.needsInput && widget.onReply != null) {
      children.add(
        Text(
          widget.question ?? '',
          key: const ValueKey('kanban-panel-question'),
          style: ts(const TypeSpec(14, 20, 400), c.textPrimary),
        ),
      );
      children.add(
        InputBox(
          child: PlainField(
            key: const ValueKey('kanban-panel-reply'),
            controller: _reply,
            focusNode: _replyFocus,
            style: ts(context.type.ui, c.textPrimary),
            placeholder: 'Reply',
            textInputAction: TextInputAction.done,
            onSubmitted: _answer,
          ),
        ),
      );
    }
    if (state == AssignState.review && widget.onAccept != null) {
      children.add(
        PrimaryButton(
          key: const ValueKey('kanban-panel-accept'),
          label: 'Accept',
          onTap: widget.onAccept!,
        ),
      );
    }
    if (state == AssignState.stalled && widget.onMarkFailed != null) {
      children.add(
        SecondaryButton(
          key: const ValueKey('kanban-panel-mark-failed'),
          label: 'Mark failed',
          onTap: widget.onMarkFailed!,
        ),
      );
    }
    if (state == AssignState.running && widget.onAddStep != null) {
      children.add(
        InputBox(
          child: PlainField(
            key: const ValueKey('kanban-panel-add-step'),
            controller: _step,
            focusNode: _stepFocus,
            style: ts(context.type.ui, c.textPrimary),
            placeholder: 'Add step',
            textInputAction: TextInputAction.done,
            onSubmitted: _add,
          ),
        ),
      );
    }
    if (widget.hasSession && widget.onOpenSession != null) {
      children.add(
        SecondaryButton(
          key: const ValueKey('kanban-panel-open-session'),
          label: 'Open session',
          onTap: widget.onOpenSession!,
        ),
      );
    }
    if (children.isEmpty) return const SizedBox.shrink();
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (final child in children)
          Padding(padding: const EdgeInsets.only(bottom: 8), child: child),
      ],
    );
  }
}

class _Prop extends StatelessWidget {
  const _Prop({required this.label, required this.child});

  final String label;
  final Widget child;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(bottom: 10),
    child: Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        SizedBox(
          width: 72,
          child: Text(
            label,
            style: ts(const TypeSpec(12.5, 20, 600), context.colors.textSecondary),
          ),
        ),
        Expanded(child: child),
      ],
    ),
  );
}

/// A property row that is a button: it takes keyboard focus (Space or Enter
/// activates it), shows a ring while focus comes from the keyboard, and is
/// one semantics node named [label] with [value]. [onTap] gets the row's
/// rectangle, to place the choice under it; null makes the row plain.
class _ChoiceRow extends StatefulWidget {
  const _ChoiceRow({
    super.key,
    required this.label,
    required this.value,
    required this.onTap,
    required this.child,
  });

  final String label;
  final String value;
  final void Function(Rect anchor)? onTap;
  final Widget child;

  @override
  State<_ChoiceRow> createState() => _ChoiceRowState();
}

class _ChoiceRowState extends State<_ChoiceRow> {
  bool _highlight = false;

  void _activate() {
    final box = context.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return;
    widget.onTap?.call(box.localToGlobal(Offset.zero) & box.size);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final visual = ExcludeSemantics(child: widget.child);
    if (widget.onTap == null) {
      return Semantics(
        container: true,
        label: widget.label,
        value: widget.value,
        child: visual,
      );
    }
    return Semantics(
      container: true,
      button: true,
      label: widget.label,
      value: widget.value,
      onTap: _activate,
      child: FocusableActionDetector(
        onShowFocusHighlight: (v) => setState(() => _highlight = v),
        shortcuts: const {
          SingleActivator(LogicalKeyboardKey.enter): ActivateIntent(),
          SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
        },
        actions: {
          ActivateIntent: CallbackAction<ActivateIntent>(
            onInvoke: (_) {
              _activate();
              return null;
            },
          ),
        },
        child: MouseRegion(
          cursor: SystemMouseCursors.click,
          child: DecoratedBox(
            position: DecorationPosition.foreground,
            decoration: BoxDecoration(
              borderRadius: Radii.field,
              border: Border.all(
                color: _highlight ? c.focus : const Color(0x00000000),
                width: 2,
              ),
            ),
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: _activate,
              child: Padding(
                padding: const EdgeInsets.symmetric(vertical: 2, horizontal: 4),
                child: visual,
              ),
            ),
          ),
        ),
      ),
    );
  }
}
