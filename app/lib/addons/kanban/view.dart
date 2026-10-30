// The kanban's widgets: the full page (title, then the board) and the compact
// board an embed draws. Both use the kit's BoardRenderer.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/note_type_registry.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/board/board_renderer.dart';
import '../../ui/board/board_source.dart';
import '../../services/addon_services.dart' show AddonServices;
import '../../ui/database/widgets.dart' show showPanel;
import '../../ui/ui.dart';
import 'keyboard_menu.dart';
import 'runner_backend.dart';
import 'runner_chip.dart';
import 'runner_fake.dart';
import 'runner_picker.dart';
import 'runner_real.dart';
import 'source.dart';

/// A kanban board. [inline] is the compact embed; otherwise the board fills
/// its parent's height. [gutter] is the side space of a full-page board.
class KanbanBoard extends StatefulWidget {
  const KanbanBoard({
    super.key,
    required this.pageId,
    this.inline = false,
    this.gutter = 0,
    this.noteTypes,
  });
  final String pageId;
  final bool inline;
  final double gutter;

  /// Draws each card through its note type. The app's registry by default.
  final NoteTypeRegistry? noteTypes;

  @override
  State<KanbanBoard> createState() => _KanbanBoardState();
}

class _KanbanBoardState extends State<KanbanBoard> {
  late KanbanBoardSource _source;

  @override
  void initState() {
    super.initState();
    _source = KanbanBoardSource(AppScope.of(context), widget.pageId);
  }

  @override
  void didUpdateWidget(KanbanBoard old) {
    super.didUpdateWidget(old);
    if (old.pageId != widget.pageId) {
      _source.dispose();
      _source = KanbanBoardSource(AppScope.of(context), widget.pageId);
    }
  }

  @override
  void dispose() {
    _source.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    _source.colors = context.colors;
    final board = ListenableBuilder(
      listenable: widget.noteTypes ?? noteTypeRegistry,
      builder: (context, _) => BoardRenderer(
        source: _source,
        inline: widget.inline,
        gutter: widget.inline ? 0 : widget.gutter,
        addCardLabel: 'New card',
        cardBuilder: _card,
        onCardMenu: (context, column, card, anchor, focus) =>
            _cardMenu(context, column, card, anchor, focus),
      ),
    );
    // Without a scope above, the board uses the agent's runners. The
    // SEED_DEMO build uses the fake runners instead.
    if (RunnerScope.maybeOf(context) != null) return board;
    final assignments = runnerSeedDemo
        ? demoRunnerAssignments
        : aiRunnerAssignments(AddonServices().aiTasks(context));
    return RunnerScope(assignments: assignments, child: board);
  }

  /// A card draws its note through the board builder, or its card builder.
  /// A type that is not installed, or a page without one, falls back to its
  /// title and first line.
  Widget _card(
    BuildContext context,
    BoardColumn column,
    BoardCard card,
    bool lifted,
  ) {
    final source = _source;
    final meta = context.watch<PagesStore>().meta(card.id);
    final pageType = meta?.pageType ?? documentType;
    final title = meta?.title ?? card.title;
    final args = NoteArgs(
      pageId: card.id,
      pageType: pageType,
      title: title,
      icon: meta?.icon,
      onOpen: () => source.openCard(context, card),
    );
    final type = (widget.noteTypes ?? noteTypeRegistry).forPage(args);
    final body = type == null
        ? _fallback(context, title, source.firstLineOf(card.id))
        : (type.boardCard ?? type.card)(context, args);
    final runners = RunnerScope.maybeOf(context);
    final shell = _CardShell(
      key: ValueKey('kanban-card-${card.id}'),
      lifted: lifted,
      menuKey: ValueKey('kanban-card-menu-${card.id}'),
      onMenu: (anchor, restoreFocus) =>
          _cardMenu(context, column, card, anchor, restoreFocus),
      body: runners == null ? body : _withChip(body, runners, card.id),
    );
    return lifted ? ExcludeFocus(child: shell) : shell;
  }

  /// The card's body, with its runner chip under it once the card has an
  /// assignment. The chip listens to the assignments, so it redraws alone.
  Widget _withChip(Widget body, RunnerAssignments runners, String cardId) =>
      Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          body,
          ListenableBuilder(
            listenable: runners,
            builder: (context, _) {
              final assignment = runners.of(cardId);
              if (assignment == null) return const SizedBox.shrink();
              return Padding(
                padding: const EdgeInsets.fromLTRB(10, 0, 10, 8),
                child: RunnerChip(assignment: assignment),
              );
            },
          ),
        ],
      );

  /// The title, then the first body line when the note has a title too (a
  /// note without one already shows its first line as its title).
  Widget _fallback(BuildContext context, String title, String firstLine) {
    final c = context.colors;
    final mobile = context.mobile;
    final showLine = title.trim().isNotEmpty && firstLine.trim().isNotEmpty;
    return Padding(
      padding: EdgeInsets.fromLTRB(10, mobile ? 10 : 8, 10, mobile ? 10 : 8),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            displayTitle(title, firstLine: firstLine),
            style: ts(
              mobile
                  ? const TypeSpec(15, 20, 500)
                  : const TypeSpec(14, 20, 500),
              c.textPrimary,
            ),
          ),
          if (showLine)
            Text(
              firstLine.trim(),
              maxLines: 2,
              overflow: TextOverflow.ellipsis,
              style: ts(const TypeSpec(13, 18, 400), c.textSecondary),
            ),
        ],
      ),
    );
  }

  /// The card's menu, from its button or a right-click. Removing only drops
  /// the reference; the note stays. A phone also moves the card to another
  /// column, where it is appended; desktop moves by dragging. [anchor] is the
  /// card's menu button. The menu takes keyboard focus on its first item;
  /// Esc closes it and gives focus back to [restoreFocus].
  void _cardMenu(
    BuildContext context,
    BoardColumn column,
    BoardCard card,
    Rect anchor,
    FocusNode? restoreFocus,
  ) {
    final source = _source;
    final phone = context.mobile;
    final runners = RunnerScope.maybeOf(context);
    final others = [
      for (final other in source.columns)
        if (other.id != column.id) other,
    ];
    showPanel<void>(
      context,
      anchor,
      (panel) => KeyboardMenu(
        restoreFocus: restoreFocus,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            MenuItem(
              key: const ValueKey('kanban-open-card'),
              label: 'Open',
              onTap: () {
                Navigator.of(panel).pop();
                source.openCard(context, card);
              },
            ),
            if (runners != null)
              MenuItem(
                key: const ValueKey('kanban-assign-card'),
                label: 'Assign to Agent',
                icon: Ico.sendHorizontal,
                onTap: () {
                  Navigator.of(panel).pop();
                  _assignCard(context, anchor, card, runners);
                },
              ),
            if (phone && others.isNotEmpty)
              MenuItem(
                key: const ValueKey('kanban-move-card'),
                label: 'Move to column…',
                onTap: () {
                  Navigator.of(panel).pop();
                  _moveMenu(context, column, card, others);
                },
              ),
            MenuItem(
              key: const ValueKey('kanban-remove-card'),
              label: 'Remove from board',
              danger: true,
              onTap: () {
                Navigator.of(panel).pop();
                source.removeCard(column.id, card.id);
              },
            ),
          ],
        ),
      ),
      width: 220,
    );
  }

  /// The runner picker for [card]: a popover under its ellipsis on a wide
  /// screen, a bottom sheet under 700 px.
  Future<void> _assignCard(
    BuildContext context,
    Rect anchor,
    BoardCard card,
    RunnerAssignments runners,
  ) async {
    if (!context.mounted) return;
    await showRunnerPicker(
      context,
      anchor: anchor,
      taskId: card.id,
      assignments: runners,
    );
  }

  /// The phone's column list for Move to column…. Choosing one appends the
  /// card to the end of that column.
  void _moveMenu(
    BuildContext anchor,
    BoardColumn from,
    BoardCard card,
    List<BoardColumn> targets,
  ) {
    if (!anchor.mounted) return;
    final source = _source;
    showPanel<void>(
      anchor,
      globalRectOf(anchor),
      (panel) => Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final target in targets)
            MenuItem(
              key: ValueKey('kanban-move-target-${target.id}'),
              label: target.title,
              onTap: () {
                Navigator.of(panel).pop();
                source.moveCardToEnd(card.id, from.id, target.id);
              },
            ),
        ],
      ),
      width: 220,
    );
  }
}

/// A card's box: its note body, with the card menu button in its own column
/// at the top right, so the button never covers the note's text. On desktop
/// the button shows while the pointer is over the card or the card has
/// keyboard focus; on a phone it is always shown. The space is kept either
/// way, so the body does not shift.
///
/// The board gives each card one keyboard stop. The context-menu key and
/// Shift+F10 on it open the card menu (see `KanbanBoard.onCardMenu`); the
/// button itself is for the pointer and for touch.
class _CardShell extends StatefulWidget {
  const _CardShell({
    super.key,
    required this.lifted,
    required this.menuKey,
    required this.onMenu,
    required this.body,
  });
  final bool lifted;
  final Key menuKey;

  /// Opens the menu under [anchor] (the menu button); the second argument is
  /// the node that gets focus back when the menu is closed with Esc.
  final void Function(Rect anchor, FocusNode? restoreFocus) onMenu;
  final Widget body;

  @override
  State<_CardShell> createState() => _CardShellState();
}

class _CardShellState extends State<_CardShell> {
  bool _hovered = false;
  final _buttonKey = GlobalKey();

  void _openMenu() {
    final box = _buttonKey.currentContext?.findRenderObject();
    if (box is! RenderBox || !box.hasSize) return;
    widget.onMenu(
      box.localToGlobal(Offset.zero) & box.size,
      BoardCardFocus.maybeOf(context)?.node,
    );
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final phone = context.mobile;
    final focused = BoardCardFocus.maybeOf(context)?.focused ?? false;
    final shown = phone || _hovered || focused;
    final content = widget.lifted
        ? widget.body
        : Row(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Expanded(child: widget.body),
              Padding(
                padding: EdgeInsets.only(top: phone ? 2 : 4, right: 4),
                child: ExcludeFocus(
                  excluding: !shown,
                  child: Opacity(
                    key: _buttonKey,
                    opacity: shown ? 1 : 0,
                    child: IgnorePointer(
                      ignoring: !shown,
                      child: IconBtn(
                        Ico.more,
                        key: widget.menuKey,
                        size: phone ? 40 : 28,
                        glyph: 20,
                        color: c.iconTertiary,
                        semanticLabel: 'Card menu',
                        onTap: _openMenu,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          );
    final box = MouseRegion(
      onEnter: (_) => setState(() => _hovered = true),
      onExit: (_) => setState(() => _hovered = false),
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: c.bgPrimary,
          borderRadius: Radii.panel,
          border: Border.all(color: c.border),
          boxShadow: widget.lifted ? c.seamHover : c.seam,
        ),
        child: content,
      ),
    );
    if (widget.lifted) return box;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (_) => _openMenu(),
      child: box,
    );
  }
}

/// The full-page kanban: the note's title, then the board, on the desk
/// background. The gutters are the database page's.
class KanbanPage extends StatelessWidget {
  const KanbanPage({
    super.key,
    required this.pageId,
    this.title = '',
    this.titleController,
    this.onTitleChanged,
    this.noteTypes,
  });
  final String pageId;
  final String title;
  final TextEditingController? titleController;
  final ValueChanged<String>? onTitleChanged;
  final NoteTypeRegistry? noteTypes;

  @override
  Widget build(BuildContext context) {
    final mobile = context.mobile;
    return ColoredBox(
      color: context.colors.desk,
      child: LayoutBuilder(
        builder: (context, box) {
          final gutter = mobile
              ? Layout.gutter
              : (box.maxWidth >= 900 ? 96.0 : 48.0);
          return Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: EdgeInsets.fromLTRB(
                  gutter,
                  mobile ? 16 : 12,
                  gutter,
                  0,
                ),
                child: _KanbanTitle(
                  title: title,
                  controller: titleController,
                  onChanged: onTitleChanged,
                ),
              ),
              SizedBox(height: mobile ? 12 : 10),
              Expanded(
                child: KanbanBoard(
                  pageId: pageId,
                  gutter: gutter,
                  noteTypes: noteTypes,
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}

class _KanbanTitle extends StatefulWidget {
  const _KanbanTitle({required this.title, this.controller, this.onChanged});
  final String title;
  final TextEditingController? controller;
  final ValueChanged<String>? onChanged;

  @override
  State<_KanbanTitle> createState() => _KanbanTitleState();
}

class _KanbanTitleState extends State<_KanbanTitle> {
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
    return ListenableBuilder(
      listenable: controller,
      builder: (context, _) => Stack(
        children: [
          if (controller.text.isEmpty)
            Positioned.fill(
              child: IgnorePointer(
                child: Text(
                  'Untitled',
                  maxLines: 1,
                  style: style.copyWith(color: c.textTertiary),
                ),
              ),
            ),
          EditableText(
            key: const ValueKey('kanban-title'),
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
          ),
        ],
      ),
    );
  }
}
