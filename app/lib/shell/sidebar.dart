// The desktop sidebar, shared by home and the editor:
// - Header: workspace and "«" (collapse, on hover). The orange New page
//   button sits at the bottom.
// - Search and Home, then Favorites (when there are any) and the page tree
//   ("Pages") from the TreeStore. Children load when a page is expanded;
//   a page without children has no chevron.
// - Each row has, on hover, "•••" (add a page inside, rename in place,
//   duplicate, move to, add to or remove from favorites, move to trash) and
//   "+" (a page inside it). Tree rows drag to reorder or into another page;
//   a blue line or highlight shows the drop.
// - Bottom: Settings (with a "Send feedback" button beside it) and Trash.
// - The right edge drags to resize (200 to 480 px). Collapsed, it takes no
//   space and [SidebarReopenButton] in the top bar brings it back;
//   [CollapsedFeedbackButton] beside it keeps "Send feedback" in reach.

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../registry/addon_registry.dart';
import '../registry/command_registry.dart';
import 'addons.dart';
import '../addons/agent/api.dart';
import '../features/settings/settings_screen.dart';
import '../features/sync/conflicts.dart' show ConflictRailGroup;
import '../features/trash/trash_panel.dart';
import '../features/updates/updates_settings.dart' show UpdateRailRow;
import '../state/app_state.dart';
import '../state/tree_store.dart';
import '../state/pages_store.dart';
import '../ui/icon_x.dart';
import '../ui/calico_art.dart';
import '../ui/space_wiggle.dart';
import '../ui/app_name.dart';
import '../ui/ui.dart';
import 'move_to.dart';
import 'navigation.dart';
import 'search.dart';

/// Shows the sidebar again when it is collapsed (desktop); nothing otherwise.
/// For the home and page top bars.
/// The tree's rows without the "No pages inside" placeholders: a page
/// without children has no chevron, so it never opens empty.
List<TreeRow> _withoutPlaceholders(List<TreeRow> rows) => [
  for (final r in rows)
    if (!r.placeholder) r,
];
List<TreeRow> _treeRows(TreeStore tree) =>
    _withoutPlaceholders(tree.visibleRows());

/// Whether this top bar stands in for a collapsed sidebar: desktop, the
/// sidebar collapsed, and the bar on the top route of the page stack.
bool _standsInForSidebar(BuildContext context) {
  final tree = context.watch<TreeStore>();
  final routePage = ModalRoute.of(context)?.settings.arguments;
  if (PageStackScope.of(context).topId !=
      (routePage is String ? routePage : null)) {
    return false;
  }
  return !context.mobile && tree.sidebarCollapsed;
}

/// Semantic label for the feedback buttons. Their visible tooltip is AppTooltip
/// with "Send feedback" and the shortcut from the registry (command feedback.send).
const feedbackButtonLabel = 'Send feedback (Ctrl+Shift+F)';

/// Runs the `feedback.send` command (the feedback sheet).
void sendFeedback(BuildContext context) =>
    commandRegistry.run('feedback.send', context);

class SidebarReopenButton extends StatelessWidget {
  const SidebarReopenButton({super.key});

  @override
  Widget build(BuildContext context) {
    final tree = context.watch<TreeStore>();
    if (!_standsInForSidebar(context)) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(right: 4),
      child: IconBtnX(
        Ix.menu,
        key: const ValueKey('sidebar-open'),
        onTap: () => tree.setSidebarCollapsed(false),
        color: context.colors.iconSecondary,
        semanticLabel: 'Open sidebar',
        command: 'sidebar.toggle',
      ),
    );
  }
}

/// "Send feedback" in the top bar while the sidebar is collapsed (desktop),
/// beside [SidebarReopenButton]; nothing otherwise.
class CollapsedFeedbackButton extends StatelessWidget {
  const CollapsedFeedbackButton({super.key});

  @override
  Widget build(BuildContext context) {
    if (!_standsInForSidebar(context)) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(right: 4),
      child: IconBtnX(
        Ix.feedback,
        key: const ValueKey('topbar-feedback'),
        onTap: () => sendFeedback(context),
        color: context.colors.iconSecondary,
        semanticLabel: feedbackButtonLabel,
        tooltip: 'Send feedback',
        command: 'feedback.send',
      ),
    );
  }
}

enum _Drop { before, after, inside }

class Sidebar extends StatefulWidget {
  const Sidebar({
    super.key,
    this.currentPageId,
    this.currentTitle,
    this.agentOpen = false,
    this.currentDestinationId,
  });

  /// The Agent screen is showing (its row is selected, Home is not).
  final bool agentOpen;
  final String? currentDestinationId;

  /// The page open in the editor, if any.
  final String? currentPageId;

  /// Its title as typed (the tree catches up when the rename is saved).
  final String? currentTitle;

  @override
  State<Sidebar> createState() => _SidebarState();
}

class _SidebarState extends State<Sidebar> {
  /// The page being renamed and the row it is renamed in (its key).
  String? _renaming, _renamingIn;
  final _renameCtl = TextEditingController();
  late final _renameFocus = FocusNode()..addListener(_renameFocusChanged);
  bool _hover = false;

  /// The row whose "•••" menu is open (its key); it stays highlighted.
  String? _menuRow;

  // Drag and drop.
  final _treeKey = GlobalKey();
  String? _dragId;
  OverlayEntry? _ghost;
  Offset _pointer = Offset.zero;
  int? _dropIndex;
  _Drop? _dropMode;

  @override
  void initState() {
    super.initState();
    final tree = context.read<TreeStore>();
    if (!tree.favoritesLoaded) tree.loadFavorites();
  }

  @override
  void dispose() {
    _ghost?.remove();
    _renameCtl.dispose();
    _renameFocus.dispose();
    super.dispose();
  }

  // ---- rename ----

  void _startRename(PageRow p, String row) {
    setState(() {
      _renaming = p.id;
      _renamingIn = row;
    });
    _renameCtl.value = TextEditingValue(
      text: p.title,
      selection: TextSelection(baseOffset: 0, extentOffset: p.title.length),
    );
    WidgetsBinding.instance.addPostFrameCallback(
      (_) => _renameFocus.requestFocus(),
    );
  }

  void _renameFocusChanged() {
    if (!_renameFocus.hasFocus && _renaming != null) _commitRename();
  }

  Future<void> _commitRename() async {
    final id = _renaming;
    if (id == null) return;
    final title = _renameCtl.text.trim();
    setState(() => _renaming = null);
    await context.app.vault.renamePage(id, title);
  }

  void _cancelRename() => setState(() => _renaming = null);

  // ---- the "•••" menu ----

  Future<void> _openMenu(PageRow p, Rect button, String row) async {
    // Beside the sidebar, top-aligned with the row, so the menu
    // does not cover the tree.
    final box = context.findRenderObject() as RenderBox;
    final right = box.localToGlobal(Offset(box.size.width, 0)).dx;
    final anchor = Rect.fromLTWH(right + 4, button.top - 8, 0, 0);
    final tree = context.read<TreeStore>();
    final fav = tree.isFavorite(p.id);
    final c = context.colors;
    setState(() => _menuRow = row);
    final action = await showPopover<String>(
      context,
      anchor,
      (ctx) => MenuList(
        width: 240,
        children: [
          MenuItem(
            key: const ValueKey('tree-menu-add'),
            icon: Ico.plus,
            label: 'Add a page inside',
            onTap: () => Navigator.of(ctx).pop('add'),
          ),
          MenuItem(
            key: const ValueKey('tree-menu-rename'),
            icon: Ico.text,
            label: 'Rename',
            onTap: () => Navigator.of(ctx).pop('rename'),
          ),
          MenuItem(
            key: const ValueKey('tree-menu-duplicate'),
            icon: Ico.duplicate,
            label: 'Duplicate',
            onTap: () => Navigator.of(ctx).pop('duplicate'),
          ),
          MenuItem(
            key: const ValueKey('tree-menu-move'),
            leading: IconX(Ix.move, size: 20, color: c.iconPrimary),
            label: 'Move to',
            onTap: () => Navigator.of(ctx).pop('move'),
          ),
          MenuItem(
            key: const ValueKey('tree-menu-favorite'),
            leading: fav
                ? IconX(Ix.starFilled, size: 20, color: c.iconPrimary)
                : null,
            icon: fav ? null : Ico.star,
            label: fav ? 'Remove from Favorites' : 'Add to Favorites',
            onTap: () => Navigator.of(ctx).pop('favorite'),
          ),
          const MenuSeparator(),
          MenuItem(
            key: const ValueKey('tree-menu-trash'),
            icon: Ico.trash,
            label: 'Move to trash',
            danger: true,
            onTap: () => Navigator.of(ctx).pop('trash'),
          ),
        ],
      ),
    );
    if (!mounted) return;
    if (_menuRow == row) setState(() => _menuRow = null);
    switch (action) {
      case 'add':
        await createPage(context, parent: p.id);
      case 'rename':
        _startRename(p, row);
      case 'duplicate':
        await context.app.vault.duplicatePage(p.id);
      case 'move':
        await showMoveTo(context, p.id, anchor: anchor);
      case 'favorite':
        await tree.setFavorite(p.id, !fav);
      case 'trash':
        await trashPage(context, p.id);
    }
  }

  void _search() => showSearchOverlay(context);

  // ---- drag and drop ----

  void _dragStart(TreeRow r, Offset global) {
    if (_renaming != null) return;
    _dragId = r.page.id;
    _pointer = global;
    final overlay = Overlay.of(context);
    final c = context.colors;
    final title = displayTitle(
      r.page.title,
      firstLine: context.read<PagesStore>().meta(r.page.id)?.firstLine ?? '',
    );
    final icon = r.page.icon;
    _ghost = OverlayEntry(
      builder: (_) => Positioned(
        left: _pointer.dx + 8,
        top: _pointer.dy - 14,
        child: IgnorePointer(
          child: Opacity(
            opacity: 0.85,
            child: MenuSurface(
              child: Container(
                height: 28,
                constraints: const BoxConstraints(maxWidth: 220),
                padding: const EdgeInsets.symmetric(horizontal: 8),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    PageIcon(icon: icon, size: 16, color: c.iconSecondary),
                    const SizedBox(width: 6),
                    Flexible(
                      child: Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: ts(
                          TypeScale.desktop.ui,
                          c.textPrimary,
                          weight: 400,
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
    overlay.insert(_ghost!);
    _dragUpdate(global);
  }

  void _dragUpdate(Offset global) {
    if (!mounted || _dragId == null) return;
    _pointer = global;
    _ghost?.markNeedsBuild();
    final box = _treeKey.currentContext?.findRenderObject() as RenderBox?;
    if (box == null) return;
    final local = box.globalToLocal(global);
    final rows = _treeRows(context.read<TreeStore>());
    int? index;
    _Drop? mode;
    if (local.dy >= 0 && local.dx >= 0 && local.dx <= box.size.width) {
      const h = Layout.sidebarRowHeight;
      final i = (local.dy / h).floor();
      if (i < rows.length) {
        final frac = (local.dy - i * h) / h;
        index = i;
        mode = frac < 0.25
            ? _Drop.before
            : (frac > 0.75 ? _Drop.after : _Drop.inside);
        if (!_canDrop(rows, i)) index = mode = null;
      }
    }
    if (index != _dropIndex || mode != _dropMode) {
      setState(() {
        _dropIndex = index;
        _dropMode = mode;
      });
    }
  }

  /// Whether the dragged page may go at row [i] (not onto itself or a page
  /// below it).
  bool _canDrop(List<TreeRow> rows, int i) {
    final from = rows.indexWhere((r) => r.page.id == _dragId);
    if (from < 0) return true;
    if (i == from) return false;
    if (i > from) {
      final d = rows[from].depth;
      for (var k = from + 1; k <= i; k++) {
        if (rows[k].depth <= d) {
          return true;
        }
      }
      return false;
    }
    return true;
  }

  Future<void> _dragEnd() async {
    if (!mounted) return;
    final id = _dragId;
    final index = _dropIndex, mode = _dropMode;
    _ghost?.remove();
    _ghost = null;
    setState(() {
      _dragId = null;
      _dropIndex = null;
      _dropMode = null;
    });
    if (id == null || index == null || mode == null) return;
    final tree = context.read<TreeStore>();
    final rows = _treeRows(tree);
    if (index >= rows.length) return;
    final r = rows[index];
    try {
      switch (mode) {
        case _Drop.inside:
          await tree.moveToEnd(id, r.page.id);
        case _Drop.after:
          await context.app.vault.movePage(
            id,
            newParent: r.page.parent,
            after: r.page.id,
          );
        case _Drop.before:
          final sibs = [
            for (final s
                in tree.loadedChildren(r.page.parent) ?? const <PageRow>[])
              if (s.id != id) s,
          ];
          final at = sibs.indexWhere((s) => s.id == r.page.id);
          await context.app.vault.movePage(
            id,
            newParent: r.page.parent,
            after: at > 0 ? sibs[at - 1].id : null,
          );
      }
    } catch (e) {
      debugPrint('move_page failed: $e');
    }
  }

  void _dragCancel() {
    if (!mounted) return;
    _ghost?.remove();
    _ghost = null;
    setState(() {
      _dragId = null;
      _dropIndex = null;
      _dropMode = null;
    });
  }

  // ---- trash ----

  void _openTrash(BuildContext row) {
    final r = globalRectOf(row);
    final box = context.findRenderObject() as RenderBox;
    final right = box.localToGlobal(Offset(box.size.width, 0)).dx;
    showTrash(context, anchor: Rect.fromLTWH(right + 4, r.bottom, 0, 0));
  }

  // ---- build ----

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tree = context.watch<TreeStore>();
    if (tree.sidebarCollapsed) return const SizedBox.shrink();
    final onHome =
        widget.currentPageId == null &&
        !widget.agentOpen &&
        widget.currentDestinationId == null;
    final destinations =
        (activeAddonContext ?? appAddonContext).shellNavigation.all
            .where((entry) => entry.sidebar)
            .toList()
          ..sort(
            (a, b) => a.order != b.order
                ? a.order.compareTo(b.order)
                : a.id.compareTo(b.id),
          );
    final agent = AgentStore.of(context.app);
    final rows = _treeRows(tree);
    final favs = tree.favorites;
    final width = tree.sidebarWidth;
    if (PageStackScope.of(context).topId != widget.currentPageId) {
      return SizedBox(width: width);
    }

    String titleOf(PageRow p) =>
        p.id == widget.currentPageId && widget.currentTitle != null
        ? widget.currentTitle!
        : p.title;

    Widget noteDrag(TreeRow r, Widget child, {bool reorder = true}) => Listener(
      onPointerDown: (event) => _pointer = event.position,
      onPointerCancel: (_) => _dragCancel(),
      child: Draggable<NoteDragPayload>(
        data: NoteDragPayload(
          pageId: r.page.id,
          title: titleOf(r.page),
          pageType: r.page.pageType,
        ),
        maxSimultaneousDrags: _renaming == null ? 1 : 0,
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: const SizedBox.shrink(),
        onDragStarted: () => _dragStart(r, _pointer),
        onDragUpdate: (d) => _dragUpdate(d.globalPosition),
        onDragEnd: (d) =>
            d.wasAccepted || !reorder ? _dragCancel() : _dragEnd(),
        child: child,
      ),
    );

    Widget treeRow(int i, TreeRow r) {
      final p = r.page;
      final drop = i == _dropIndex ? _dropMode : null;
      Widget row = noteDrag(
        r,
        _TreeRowView(
          key: ValueKey('tree-${p.id}'),
          prefix: 'tree',
          page: p,
          depth: r.depth,
          showToggle: true,
          title: titleOf(p),
          selected: p.id == widget.currentPageId,
          dragging: p.id == _dragId,
          expanded: tree.isExpanded(p.id),
          menuOpen: _menuRow == 'tree-${p.id}',
          renaming: _renaming == p.id && _renamingIn == 'tree-${p.id}',
          renameCtl: _renameCtl,
          renameFocus: _renameFocus,
          onRenameSubmit: _commitRename,
          onRenameCancel: _cancelRename,
          onTap: p.id == widget.currentPageId
              ? null
              : () => openPage(context, p.id, p.title),
          onToggle: () => tree.toggle(p.id),
          onAdd: () => createPage(context, parent: p.id),
          onMenu: (rect) => _openMenu(p, rect, 'tree-${p.id}'),
        ),
      );
      if (drop != null) {
        row = _DropIndicator(mode: drop, depth: r.depth, child: row);
      }
      return row;
    }

    // Favorites expand like the tree, with their own expanded pages. A row
    // inside a favorite is keyed by that favorite too (a page can show up
    // under more than one).
    Widget favRow(TreeRow r) {
      final p = r.page;
      final k = r.depth == 0 ? 'fav-${p.id}' : 'fav-${r.root}-${p.id}';
      return noteDrag(
        r,
        _TreeRowView(
          key: ValueKey(k),
          prefix: 'fav',
          page: p,
          depth: r.depth,
          showToggle: true,
          title: titleOf(p),
          selected: p.id == widget.currentPageId,
          dragging: p.id == _dragId,
          expanded: tree.isExpanded(p.id, fav: true),
          menuOpen: _menuRow == k,
          renaming: _renaming == p.id && _renamingIn == k,
          renameCtl: _renameCtl,
          renameFocus: _renameFocus,
          onRenameSubmit: _commitRename,
          onRenameCancel: _cancelRename,
          onTap: p.id == widget.currentPageId
              ? null
              : () => openPage(context, p.id, p.title),
          onToggle: () => tree.toggle(p.id, fav: true),
          onAdd: () => createPage(context, parent: p.id),
          onMenu: (rect) => _openMenu(p, rect, k),
        ),
        reorder: false,
      );
    }

    final list = ListView(
      padding: EdgeInsets.zero,
      children: [
        if (favs.isNotEmpty) ...[
          _SectionLabel('Favorites', key: const ValueKey('sidebar-favorites')),
          for (final r in _withoutPlaceholders(tree.favoriteRows())) favRow(r),
          const SizedBox(height: 12),
        ],
        const _SectionLabel('Pages'),
        Column(
          key: _treeKey,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [for (var i = 0; i < rows.length; i++) treeRow(i, rows[i])],
        ),
        const SizedBox(height: 8),
      ],
    );

    return MouseRegion(
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: SizedBox(
        key: const ValueKey('sidebar'),
        width: width,
        child: Stack(
          children: [
            Positioned.fill(
              child: Container(
                decoration: BoxDecoration(
                  color: c.rail,
                  border: Border(right: BorderSide(color: c.railLine)),
                ),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    const SizedBox(height: 6),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 8),
                      child: SizedBox(
                        height: 32,
                        child: Row(
                          children: [
                            const SizedBox(width: 8),
                            SizedBox(
                              width: 26,
                              height: 26,
                              child: const CalicoMark(size: 26),
                            ),
                            const SizedBox(width: 10),
                            Expanded(
                              child: Text(
                                appName,
                                maxLines: 1,
                                style: tsDisplay(
                                  const TypeSpec(19, 23, 660),
                                  c.railInk,
                                ),
                              ),
                            ),
                            if (_hover)
                              IconBtnX(
                                Ix.collapse,
                                key: const ValueKey('sidebar-collapse'),
                                onTap: () => tree.setSidebarCollapsed(true),
                                size: 28,
                                glyph: 18,
                                color: c.railMuted,
                                semanticLabel: 'Close sidebar',
                                command: 'sidebar.toggle',
                              ),
                          ],
                        ),
                      ),
                    ),
                    const SizedBox(height: 4),
                    _NavRow(
                      key: const ValueKey('sidebar-search'),
                      icon: Ico.search,
                      label: 'Search',
                      onTap: _search,
                    ),
                    _NavRow(
                      key: const ValueKey('sidebar-home'),
                      icon: Ico.home,
                      label: 'Home',
                      selected: onHome,
                      onTap: () => goHome(context),
                    ),
                    for (final entry in destinations)
                      _NavRow(
                        key: ValueKey('sidebar-${entry.id}'),
                        icon: entry.icon,
                        label: entry.label,
                        selected: widget.currentDestinationId == entry.id,
                        onTap: () => entry.open(context),
                      ),
                    ListenableBuilder(
                      listenable: agent,
                      builder: (context, _) => agent.enabled
                          ? _NavRow(
                              key: const ValueKey('sidebar-agent'),
                              icon: Ico.comment,
                              label: 'Agent',
                              selected: widget.agentOpen,
                              onTap: () => openAgent(context),
                            )
                          : const SizedBox.shrink(),
                    ),
                    const SizedBox(height: 14),
                    Expanded(child: list),
                    Padding(
                      padding: const EdgeInsets.fromLTRB(10, 6, 10, 8),
                      child: KeyedSubtree(
                        key: const ValueKey('sidebar-new-page-row'),
                        child: PrimaryButton(
                          key: const ValueKey('new-page'),
                          label: 'New page',
                          height: 42,
                          icon: Ico.plus,
                          onTap: () => createPage(context),
                        ),
                      ),
                    ),
                    const ConflictRailGroup(),
                    const UpdateRailRow(),
                    const SizedBox(height: 4),
                    Row(
                      children: [
                        Expanded(
                          child: _NavRow(
                            key: const ValueKey('sidebar-settings'),
                            icon: Ico.settings,
                            label: 'Settings',
                            onTap: () => showSettings(context),
                          ),
                        ),
                        Padding(
                          padding: const EdgeInsets.only(right: 8),
                          child: IconBtnX(
                            Ix.feedback,
                            key: const ValueKey('sidebar-feedback'),
                            size: Layout.sidebarRowHeight - 2,
                            glyph: 18,
                            color: c.railMuted,
                            onTap: () => sendFeedback(context),
                            semanticLabel: feedbackButtonLabel,
                            tooltip: 'Send feedback',
                            command: 'feedback.send',
                          ),
                        ),
                      ],
                    ),
                    Builder(
                      builder: (row) => _NavRow(
                        key: const ValueKey('sidebar-trash'),
                        icon: Ico.trash,
                        label: 'Trash',
                        onTap: () => _openTrash(row),
                      ),
                    ),
                    const SizedBox(height: 8),
                  ],
                ),
              ),
            ),
            Positioned(
              right: 0,
              top: 0,
              bottom: 0,
              width: 6,
              child: _ResizeHandle(
                key: const ValueKey('sidebar-resize'),
                onDrag: (dx) => tree.setSidebarWidth(tree.sidebarWidth + dx),
                onReset: () => tree.setSidebarWidth(TreeStore.defaultWidth),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel(this.text, {super.key});
  final String text;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 0, 16, 0),
      child: SizedBox(
        height: 30,
        child: Align(
          alignment: Alignment.centerLeft,
          child: Text(text, style: tsRaw(12, 16, 700, c.railMuted)),
        ),
      ),
    );
  }
}

/// The 2 px accent line (before/after) or the highlight (inside) of a drop.
class _DropIndicator extends StatelessWidget {
  const _DropIndicator({
    required this.mode,
    required this.depth,
    required this.child,
  });
  final _Drop mode;
  final int depth;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final left = 8.0 + 4 + 12.0 * depth + 22;
    return Stack(
      children: [
        if (mode == _Drop.inside)
          Positioned.fill(
            key: const ValueKey('tree-drop-inside'),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
              child: DecoratedBox(
                decoration: BoxDecoration(
                  color: c.bgSelectedBlock,
                  borderRadius: BorderRadius.circular(Radii.md),
                ),
              ),
            ),
          ),
        child,
        if (mode != _Drop.inside)
          Positioned(
            key: const ValueKey('tree-drop-line'),
            left: left,
            right: 8,
            top: mode == _Drop.before ? 0 : null,
            bottom: mode == _Drop.after ? 0 : null,
            height: 2,
            child: ColoredBox(color: c.accent.withValues(alpha: 0.6)),
          ),
      ],
    );
  }
}

class _ResizeHandle extends StatefulWidget {
  const _ResizeHandle({super.key, required this.onDrag, required this.onReset});
  final ValueChanged<double> onDrag;
  final VoidCallback onReset;

  @override
  State<_ResizeHandle> createState() => _ResizeHandleState();
}

class _ResizeHandleState extends State<_ResizeHandle> {
  bool _hover = false, _down = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // The width follows the pointer from where it went down (the drag
        // slop is not lost).
        dragStartBehavior: DragStartBehavior.down,
        onHorizontalDragStart: (_) => setState(() => _down = true),
        onHorizontalDragUpdate: (d) => widget.onDrag(d.delta.dx),
        onHorizontalDragEnd: (_) => setState(() => _down = false),
        onDoubleTap: widget.onReset,
        child: Align(
          alignment: Alignment.centerRight,
          child: Container(
            width: 2,
            color: _hover || _down ? c.borderStrong : const Color(0x00000000),
          ),
        ),
      ),
    );
  }
}

class _NavRow extends StatefulWidget {
  const _NavRow({
    super.key,
    required this.icon,
    required this.label,
    required this.onTap,
    this.selected = false,
  });
  final Ico icon;
  final String label;
  final VoidCallback onTap;
  final bool selected;

  @override
  State<_NavRow> createState() => _NavRowState();
}

class _NavRowState extends State<_NavRow> {
  bool _hover = false;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
      child: Hover(
        onTap: widget.onTap,
        selected: widget.selected,
        selectedColor: railSelected(c),
        hoverColor: railHover(c),
        onHoverChanged: (value) => setState(() => _hover = value),
        borderRadius: Radii.ctl,
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: SizedBox(
          height: Layout.sidebarRowHeight - 2,
          child: Row(
            children: [
              SizedBox(
                width: 20,
                height: 20,
                child: Center(
                  child: Icon20(
                    widget.icon,
                    size: 18,
                    color: widget.selected || _hover ? c.railInk : c.railMuted,
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Text(
                  widget.label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(
                    ty.ui,
                    widget.selected || _hover ? c.railInk : c.railMuted,
                    weight: widget.selected ? 700 : 400,
                  ),
                ),
              ),
              if (widget.label == 'Search')
                Opacity(
                  opacity: 0.7,
                  child: Text(
                    'Ctrl K',
                    style: ts(
                      const TypeSpec(11, 15, 400),
                      c.railMuted,
                      mono: true,
                    ),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

class _TreeRowView extends StatefulWidget {
  const _TreeRowView({
    super.key,
    required this.prefix,
    required this.page,
    required this.depth,
    required this.showToggle,
    required this.title,
    required this.selected,
    required this.dragging,
    required this.expanded,
    this.menuOpen = false,
    required this.renaming,
    required this.renameCtl,
    required this.renameFocus,
    required this.onRenameSubmit,
    required this.onRenameCancel,
    required this.onTap,
    required this.onToggle,
    required this.onAdd,
    required this.onMenu,
  });

  /// Key prefix of the row's buttons ('tree' or 'fav').
  final String prefix;
  final PageRow page;
  final int depth;
  final bool showToggle;
  final String title;
  final bool selected, dragging, expanded, renaming;

  /// Its "•••" menu is open: keep the hover look and the buttons.
  final bool menuOpen;
  final TextEditingController renameCtl;
  final FocusNode renameFocus;
  final VoidCallback onRenameSubmit, onRenameCancel, onToggle, onAdd;
  final VoidCallback? onTap;
  final ValueChanged<Rect> onMenu;

  @override
  State<_TreeRowView> createState() => _TreeRowViewState();
}

class _TreeRowViewState extends State<_TreeRowView> {
  bool _hover = false;

  void _menu() {
    widget.onMenu(globalRectOf(context));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final p = widget.page;
    final id = p.id;
    final pages = context.watch<PagesStore>();
    final ancestors = pages.ancestors(id);
    final color = ancestors.isEmpty
        ? pages.meta(id)?.color
        : ancestors.first.color;
    final patch = spacePatch(
      color,
      c.dark ? Brightness.dark : Brightness.light,
    );
    final tilePatch = p.parent == null
        ? spacePatch(
            color,
            c.dark ? Brightness.dark : Brightness.light,
            rail: true,
          )
        : null;
    final pre = widget.prefix;
    final showActions = (_hover || widget.menuOpen) && !widget.renaming;

    final Widget label = widget.renaming
        ? CallbackShortcuts(
            bindings: {
              const SingleActivator(LogicalKeyboardKey.escape):
                  widget.onRenameCancel,
            },
            child: PlainField(
              key: const ValueKey('tree-rename-field'),
              controller: widget.renameCtl,
              focusNode: widget.renameFocus,
              style: ts(ty.ui, c.railInk),
              placeholder: 'Untitled',
              onSubmitted: (_) => widget.onRenameSubmit(),
            ),
          )
        : PageLabel(
            id: id,
            title: widget.title,
            tertiary: c.railMuted,
            style: ts(
              ty.ui,
              widget.selected || _hover || widget.menuOpen
                  ? c.railInk
                  : c.railMuted,
              weight: widget.selected ? 700 : 400,
            ),
          );

    final icon = tilePatch != null
        ? SpaceWiggle(
            child: SpaceIconTile(
              key: ValueKey('rail-space-tile-$id'),
              icon: p.icon,
              patch: tilePatch,
              size: 22,
              glyphSize: 16,
            ),
          )
        : SizedBox(
            width: 20,
            height: 20,
            child: Center(
              child: PageIcon(icon: p.icon, size: 18, color: c.railMuted),
            ),
          );

    return Opacity(
      opacity: widget.dragging ? 0.4 : 1,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 1),
        child: Stack(
          children: [
            Hover(
              onTap: widget.onTap,
              onSecondaryTap: _menu,
              onHoverChanged: (h) => setState(() => _hover = h),
              selected: widget.selected || widget.menuOpen,
              selectedColor: widget.selected ? railSelected(c) : c.rail2,
              hoverColor: railHover(c),
              borderRadius: Radii.ctl,
              padding: EdgeInsets.only(left: 4 + 12.0 * widget.depth, right: 4),
              child: SizedBox(
                height: Layout.sidebarRowHeight - 2,
                child: Row(
                  children: [
                    if (widget.showToggle && p.childCount > 0) ...[
                      Semantics(
                        button: true,
                        expanded: widget.expanded,
                        label: widget.expanded ? 'Collapse' : 'Expand',
                        excludeSemantics: true,
                        child: AppTooltip(
                          label: widget.expanded ? 'Collapse' : 'Expand',
                          child: Hover(
                            key: ValueKey('$pre-toggle-$id'),
                            onTap: widget.onToggle,
                            radius: Radii.sm,
                            child: SizedBox(
                              width: 20,
                              height: 20,
                              child: Center(
                                child: Icon20(
                                  widget.expanded
                                      ? Ico.chevronDown
                                      : Ico.chevronRight,
                                  size: 12,
                                  color: c.railMuted,
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                      const SizedBox(width: 2),
                    ] else if (widget.showToggle)
                      // A leaf keeps the chevron's room so icons line up.
                      const SizedBox(width: 22)
                    else
                      const SizedBox(width: 4),
                    icon,
                    const SizedBox(width: 6),
                    Expanded(
                      child: Row(children: [Flexible(child: label)]),
                    ),
                    if (showActions) ...[
                      IconBtn(
                        Ico.more,
                        key: ValueKey('$pre-more-$id'),
                        onTap: _menu,
                        size: 20,
                        glyph: 14,
                        radius: Radii.sm,
                        color: c.railMuted,
                        semanticLabel: 'Page actions',
                      ),
                      IconBtn(
                        Ico.plus,
                        key: ValueKey('$pre-add-$id'),
                        onTap: widget.onAdd,
                        size: 20,
                        glyph: 14,
                        radius: Radii.sm,
                        color: c.railMuted,
                        semanticLabel: 'Add a page inside',
                      ),
                    ],
                  ],
                ),
              ),
            ),
            if (widget.selected)
              Positioned(
                left: 0,
                top: 0,
                bottom: 0,
                width: 3,
                child: IgnorePointer(
                  child: ColoredBox(
                    key: ValueKey('rail-current-bar-$id'),
                    color: patch?.fill ?? c.accent,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// The current row on the rail: a lighter fill than hover, with the bar.
Color railSelected(AppColors c) =>
    Color.alphaBlend(c.railInk.withValues(alpha: .10), c.rail2);

/// A hovered row on the rail: a fainter fill than the current row.
Color railHover(AppColors c) => c.rail2.withValues(alpha: .55);
