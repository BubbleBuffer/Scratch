// The trash: pages moved to the trash, with search, restore and delete
// forever (after a confirm). Each row shows where the page was (its old
// parent's path) when that page still exists. A popover on desktop, a
// full-screen page on mobile. The list reloads from the change feed.

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/icon_x.dart';
import '../../ui/ui.dart';

/// Shows the trash next to [anchor], or as a screen on a compact screen.
Future<void> showTrash(BuildContext context, {Rect? anchor}) async {
  if (context.sizeClass.isCompact) {
    await pushScreen<void>(
        context, (_) => const ScreenScaffold(title: 'Trash', body: TrashPanel(key: ValueKey('trash-panel'))),
        name: '/trash');
    return;
  }
  if (anchor == null) {
    await showModalPanel<void>(context, (_) => const TrashPanel(key: ValueKey('trash-panel')), width: TrashSize.width, height: TrashSize.panel);
    return;
  }
  await showPopover<void>(
    context,
    anchor,
    (_) => const MenuSurface(
      width: TrashSize.width,
      child: SizedBox(height: TrashSize.popover, child: TrashPanel(key: ValueKey('trash-panel'))),
    ),
  );
}

class TrashPanel extends StatefulWidget {
  const TrashPanel({super.key});

  @override
  State<TrashPanel> createState() => _TrashPanelState();
}

class _TrashPanelState extends State<TrashPanel> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  List<TrashedPage>? _items;
  late final ChangeFeed _feed = context.app.feed;
  bool _disposed = false;
  int _gen = 0;

  @override
  void initState() {
    super.initState();
    _feed.addListener(_onChanges);
    _load();
  }

  @override
  void dispose() {
    _disposed = true;
    _feed.removeListener(_onChanges);
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _onChanges(ChangeBatch b) => _load();

  Future<void> _load() async {
    final g = ++_gen;
    try {
      final items = await context.app.vault.listTrash();
      if (_disposed || g != _gen) return;
      setState(() => _items = items);
    } catch (e) {
      debugPrint('list_trash failed: $e');
    }
  }

  Future<void> _restore(String id) async {
    await context.app.vault.restorePage(id);
    await _load();
  }

  Future<void> _purge(TrashedPage t) async {
    final ok = await showConfirm(
      context,
      title: 'Delete "${displayTitle(t.page.title, firstLine: _firstLine(context.read<PagesStore>(), t.page.id))}" forever?',
      message: 'The page and the pages inside it are deleted from this vault. This cannot be undone.',
      confirmLabel: 'Delete forever',
      danger: true,
    );
    if (!ok || !mounted) return;
    await context.app.vault.run((v) => v.purgePage(id: t.page.id), writes: true);
    await _load();
  }

  /// A trashed page's first body line, from its metadata (loaded on first
  /// use; the rows rebuild when it arrives). Empty when it has none.
  String _firstLine(PagesStore pages, String id) => pages.meta(id)?.firstLine ?? '';

  /// The old parent's path ("A / B"), when the parent still exists.
  String? _path(PagesStore pages, String? parent) {
    if (parent == null) return null;
    final m = pages.meta(parent);
    if (m == null || m.missing) return null;
    final chain = [...pages.ancestors(parent), m];
    return chain.map((p) => displayTitle(p.title, firstLine: p.firstLine)).join(' / ');
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    // A screen of its own on a compact screen, a panel otherwise.
    final m = context.sizeClass.isCompact;
    final pages = context.watch<PagesStore>();
    final q = _ctl.text.trim().toLowerCase();
    final items = [
      for (final t in _items ?? const <TrashedPage>[])
        if (q.isEmpty || t.page.title.toLowerCase().contains(q)) t,
    ];
    final gutter = m ? Layout.gutter : Space.md;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: EdgeInsets.fromLTRB(gutter, m ? Space.xxs : Space.md, gutter, Space.xs),
        child: InputBox(
          height: context.metrics.field.height,
          child: Row(children: [
            Icon20(Ico.search, size: m ? IconSize.md : IconSize.xs, color: c.iconTertiary),
            const Gap(Space.xs),
            Expanded(
              child: PlainField(
                key: const ValueKey('trash-search'),
                controller: _ctl,
                focusNode: _focus,
                autofocus: !m,
                style: ts(m ? ty.body : ty.ui, c.textPrimary, weight: Weight.regular),
                placeholder: 'Search pages in Trash',
                onChanged: (_) => setState(() {}),
              ),
            ),
          ]),
        ),
      ),
      Expanded(
        child: _items == null
            ? const SizedBox.shrink()
            : items.isEmpty
                ? Center(
                    child: Padding(
                      padding: const EdgeInsets.all(Space.xl),
                      child: Column(mainAxisSize: MainAxisSize.min, children: [
                        Icon20(Ico.trash, size: IconSize.xxl, color: c.iconTertiary),
                        const Gap(Space.sm),
                        Text(q.isEmpty ? 'Trash is empty' : 'No matches',
                            key: const ValueKey('trash-empty'), style: ts(ty.ui, c.textSecondary, weight: Weight.regular)),
                      ]),
                    ),
                  )
                : ListView.builder(
                    padding: EdgeInsets.symmetric(horizontal: m ? 0 : Space.xxs, vertical: Space.hair),
                    itemCount: items.length,
                    itemBuilder: (context, i) => _row(context, items[i], _path(pages, items[i].page.parent),
                        _firstLine(pages, items[i].page.id)),
                  ),
      ),
      Container(
        padding: EdgeInsets.symmetric(horizontal: gutter, vertical: Space.sm),
        decoration: BoxDecoration(border: Border(top: BorderSide(color: c.borderSubtle))),
        child: Text('Pages in the trash stay until you delete them forever.',
            style: ts(ty.uiSmall, c.textTertiary, weight: Weight.regular)),
      ),
    ]);
  }

  Widget _row(BuildContext context, TrashedPage t, String? path, String firstLine) {
    final c = context.colors;
    final ty = context.type;
    final m = context.touch;
    final p = t.page;
    return PanelRow(
      key: ValueKey('trash-row-${p.id}'),
      onTap: () {},
      padding: EdgeInsets.symmetric(horizontal: m ? Layout.gutter : Space.xs),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: m ? TrashSize.rowTouch : (path == null ? TrashSize.row : TrashSize.rowWithPath)),
        child: Row(children: [
          PageIcon(icon: p.icon, size: (m ? IconSize.xl : IconSize.md).px, color: c.iconSecondary),
          Gap(m ? Space.md : Space.sm),
          Expanded(
            child: Column(mainAxisAlignment: MainAxisAlignment.center, crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(displayTitle(p.title, firstLine: firstLine),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: ts(m ? ty.body : ty.ui, c.textPrimary, weight: Weight.regular)),
              if (path != null)
                Text(path,
                    key: ValueKey('trash-path-${p.id}'),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(ty.uiSmall, c.textTertiary, weight: Weight.regular)),
            ]),
          ),
          Text(relativeTime(t.trashedMs.toInt()), style: ts(ty.uiSmall, c.textTertiary, weight: Weight.regular)),
          const Gap(Space.xs),
          IconBtnX(Ix.restore,
              key: ValueKey('trash-restore-${p.id}'),
              onTap: () => _restore(p.id),
              size: m ? ControlSize.xl : ControlSize.xs,
              glyph: m ? IconSize.lg : IconSize.sm,
              color: c.iconSecondary,
              semanticLabel: 'Restore'),
          IconBtn(Ico.trash,
              key: ValueKey('trash-delete-${p.id}'),
              onTap: () => _purge(t),
              size: m ? ControlSize.xl : ControlSize.xs,
              glyph: m ? IconSize.lg : IconSize.sm,
              color: c.iconSecondary,
              semanticLabel: 'Delete forever'),
        ]),
      ),
    );
  }
}
