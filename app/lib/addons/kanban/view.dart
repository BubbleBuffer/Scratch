// The kanban's widgets: the full page (title, then the board) and the compact
// board an embed draws. Both use the kit's BoardRenderer.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/note_type_registry.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/board/board_renderer.dart';
import '../../ui/board/board_source.dart';
import '../../ui/database/widgets.dart' show showPanel;
import '../../ui/ui.dart';
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
    return ListenableBuilder(
      listenable: widget.noteTypes ?? noteTypeRegistry,
      builder: (context, _) => BoardRenderer(
        source: _source,
        inline: widget.inline,
        gutter: widget.inline ? 0 : widget.gutter,
        addCardLabel: 'New card',
        cardBuilder: _card,
      ),
    );
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
    final shell = _CardShell(
      key: ValueKey('kanban-card-${card.id}'),
      lifted: lifted,
      menuKey: ValueKey('kanban-card-menu-${card.id}'),
      onMenu: () => _cardMenu(context, column, card),
      body: body,
    );
    if (lifted) return ExcludeFocus(child: shell);
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (_) => _cardMenu(context, column, card),
      child: shell,
    );
  }

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
  /// column, where it is appended; desktop moves by dragging.
  void _cardMenu(BuildContext context, BoardColumn column, BoardCard card) {
    final source = _source;
    final phone = context.mobile;
    final others = [
      for (final other in source.columns)
        if (other.id != column.id) other,
    ];
    showPanel<void>(
      context,
      globalRectOf(context),
      (panel) => Column(
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
      width: 220,
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
/// the button shows while the pointer is over the card; on a phone it is
/// always shown. The space is kept either way, so the body does not shift.
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
  final VoidCallback onMenu;
  final Widget body;

  @override
  State<_CardShell> createState() => _CardShellState();
}

class _CardShellState extends State<_CardShell> {
  bool _hovered = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final phone = context.mobile;
    final shown = phone || _hovered;
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
                        onTap: widget.onMenu,
                      ),
                    ),
                  ),
                ),
              ),
            ],
          );
    return MouseRegion(
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
