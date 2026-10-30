// The panel a board card opens: its title, status (the column it is in), the
// runner chip, an Assign to Agent button that opens the Run on picker, and a
// link to the full note. On desktop it sits beside the board; on a phone it
// is a bottom sheet. Esc closes it.

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

  /// True in a bottom sheet, which has its own surface and no close button.
  final bool sheet;

  @override
  State<KanbanCardPanel> createState() => _KanbanCardPanelState();
}

class _KanbanCardPanelState extends State<KanbanCardPanel> {
  final _scope = FocusScopeNode(debugLabel: 'kanban card panel');
  final _assignKey = GlobalKey();

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) _scope.nextFocus();
    });
  }

  @override
  void dispose() {
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
                      Ico.x,
                      key: const ValueKey('kanban-panel-close'),
                      semanticLabel: 'Close',
                      onTap: widget.onClose,
                    ),
                  ),
                Text(
                  widget.title,
                  key: const ValueKey('kanban-panel-title'),
                  style: tsDisplay(const TypeSpec(22, 28, 700), c.textPrimary),
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
                  child: Text(
                    widget.status,
                    key: const ValueKey('kanban-panel-status'),
                    style: ts(const TypeSpec(14, 20, 400), c.textPrimary),
                  ),
                ),
                if (runners != null)
                  _Prop(
                    label: 'Runner',
                    child: ListenableBuilder(
                      listenable: runners,
                      builder: (context, _) {
                        final a = runners.of(widget.cardId);
                        return a == null
                            ? Text(
                                'Not assigned',
                                style: ts(
                                  const TypeSpec(14, 20, 400),
                                  c.textSecondary,
                                ),
                              )
                            : RunnerChip(assignment: a);
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
                              expand: true,
                              onTap: _assign,
                            ),
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
