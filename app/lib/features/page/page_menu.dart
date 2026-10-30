// The page's "•••" menu:
// small text and full width switches, lock, favourites, copy link, share (with a hub
// sign-in), duplicate, move to, Ask AI (about the page), undo, page history, deleted pages, delete, import
// and export, then Send feedback. The switches apply in place; the other rows close the menu
// and hand their action to the editor. A popover on desktop, a sheet on
// mobile.

import 'package:flutter/widgets.dart';

import '../../state/app_state.dart';
import '../../state/pages_store.dart';
import '../../ui/icon_x.dart';
import '../../ui/ui.dart';
import '../ai/ask_ai.dart' show AiMark;
import '../settings/controls.dart' show Toggle;
import '../share/share_popup.dart' show shareClientOf;

/// The Share item's label: "Share · 1 link", "Share · 2 links". Plain "Share"
/// while the count is unknown (loading or failed) and with no links.
String shareMenuLabel(int? links) => (links == null || links == 0)
    ? 'Share'
    : 'Share · $links ${links == 1 ? 'link' : 'links'}';

enum PageMenuAction { favorite, copyLink, share, duplicate, moveTo, askAi, undo, history, trash, delete, import, export, feedback }

Future<PageMenuAction?> showPageMenu(BuildContext context, Rect? anchor,
    {required String pageId, required bool favorite, required bool canUndo, bool canAskAi = false, bool canShare = false}) {
  Widget body(BuildContext ctx) => PageMenu(
        pageId: pageId,
        favorite: favorite,
        canUndo: canUndo,
        canAskAi: canAskAi,
        canShare: canShare,
        sheet: context.mobile,
        onAction: (a) => Navigator.of(ctx).pop(a),
      );
  if (context.mobile || anchor == null) {
    return showSheet<PageMenuAction>(context, (ctx) => SheetSurface(children: [body(ctx)]));
  }
  return showPopover<PageMenuAction>(
      context, anchor, (ctx) => MenuSurface(width: 265, padding: const EdgeInsets.all(6), child: body(ctx)));
}

class PageMenu extends StatefulWidget {
  const PageMenu({
    super.key,
    required this.pageId,
    required this.favorite,
    required this.canUndo,
    required this.onAction,
    this.sheet = false,
    this.canAskAi = false,
    this.canShare = false,
  });
  final String pageId;
  final bool favorite, canUndo, sheet;

  /// Shows "Share" (the share popup; only with a hub sign-in, not for databases).
  final bool canShare;

  /// Shows "Ask AI" (an answer about the page, written at its end).
  final bool canAskAi;
  final ValueChanged<PageMenuAction> onAction;

  @override
  State<PageMenu> createState() => _PageMenuState();
}

class _PageMenuState extends State<PageMenu> {
  bool _colour = false;
  bool _saving = false;
  String? _error;

  /// The page's share links on the hub; null while loading or on error.
  int? _shareCount;
  bool _counted = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_counted || !widget.canShare) return;
    _counted = true;
    _countLinks();
  }

  /// One GET when the menu opens: the Share item then says how many links.
  Future<void> _countLinks() async {
    final client = shareClientOf(context);
    if (client == null) return;
    try {
      final links = await client.list(widget.pageId);
      if (mounted) setState(() => _shareCount = links.length);
    } catch (_) {
      // The item keeps its plain label.
    }
  }

  @override
  Widget build(BuildContext context) {
    final pageId = widget.pageId;
    final favorite = widget.favorite, canUndo = widget.canUndo, sheet = widget.sheet;
    final onAction = widget.onAction;
    final c = context.colors;
    final ty = context.type;
    final pages = context.watch<PagesStore>();
    final prefs = pages.prefs(pageId);
    final meta = pages.meta(pageId);
    final m = sheet;
    final rowH = m ? 48.0 : 28.0;
    final textStyle = ts(m ? ty.body : ty.ui, c.textPrimary, weight: 400);
    final iconSize = m ? 22.0 : 18.0;

    Widget row(String key, Widget icon, String label, VoidCallback? onTap,
        {String? shortcut, Widget? trailing, bool danger = false}) {
      return Hover(
        key: ValueKey('page-menu-$key'),
        onTap: onTap,
        radius: m ? 0 : Radii.md,
        padding: EdgeInsets.symmetric(horizontal: m ? Layout.gutter : 8),
        child: SizedBox(
          height: rowH,
          child: Row(children: [
            SizedBox(width: m ? 22 : 20, child: Center(child: icon)),
            SizedBox(width: m ? 14 : 8),
            Expanded(
              child: Text(label,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: danger ? textStyle.copyWith(color: c.inlineCodeText) : textStyle),
            ),
            if (shortcut != null && !m) Text(shortcut, style: ts(ty.uiSmall, c.textTertiary, weight: 400)),
            ?trailing,
          ]),
        ),
      );
    }

    Widget ico(Ico i, {bool danger = false}) => Icon20(i, size: iconSize, color: danger ? c.inlineCodeText : c.iconPrimary);
    Widget icx(Ix i) => IconX(i, size: iconSize, color: c.iconPrimary);
    Widget sep() => m
        ? Padding(padding: const EdgeInsets.symmetric(vertical: 4), child: HLine(color: c.borderSubtle))
        : const MenuSeparator();

    void set(void Function(PagePrefs p) f) => pages.updatePrefs(pageId, f);

    if (_colour) {
      Future<void> pick(String? color) async {
        if (_saving) return;
        setState(() => _saving = true);
        try {
          await context.app.vault.run((v) => v.setPageColor(page: pageId, color: color), writes: true);
          if (context.mounted) setState(() => _colour = false);
        } catch (e) {
          if (context.mounted) setState(() => _error = 'Could not change colour: $e');
        } finally {
          if (context.mounted) setState(() => _saving = false);
        }
      }
      return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        row('colour-back', ico(Ico.chevronLeft), 'Colour', () => setState(() => _colour = false)),
        Padding(
          padding: EdgeInsets.symmetric(horizontal: m ? Layout.gutter : 8, vertical: 8),
          child: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            for (final name in spaceColorNames)
              Hover(
                key: ValueKey('page-colour-$name'),
                onTap: _saving ? null : () => pick(name),
                radius: Radii.md,
                child: Semantics(label: name, button: true, selected: meta?.color == name, child: Container(
                  width: m ? 40 : 32, height: m ? 40 : 32,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    borderRadius: BorderRadius.circular(Radii.md),
                    border: meta?.color == name ? Border.all(color: c.accent) : null,
                  ),
                  child: Container(width: 20, height: 20, decoration: BoxDecoration(
                    shape: BoxShape.circle, color: spaceColor(name, c.dark ? Brightness.dark : Brightness.light),
                  )),
                )),
              ),
          ]),
        ),
        row('colour-none', ico(Ico.close), 'None', _saving ? null : () => pick('none')),
        if (_error != null) Padding(padding: const EdgeInsets.all(8), child: Text(_error!, style: ts(ty.uiSmall, c.inlineCodeText))),
      ]);
    }
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      row('small-text', icx(Ix.textSize), 'Small text', () => set((p) => p.smallText = !p.smallText),
          trailing: Toggle(value: prefs.smallText, onChanged: (v) => set((p) => p.smallText = v))),
      if (!m)
        row('full-width', icx(Ix.fullWidth), 'Full width', () => set((p) => p.fullWidth = !p.fullWidth),
            trailing: Toggle(value: prefs.fullWidth, onChanged: (v) => set((p) => p.fullWidth = v))),
      sep(),
      row('lock', icx(Ix.lock), prefs.locked ? 'Unlock page' : 'Lock page', () => set((p) => p.locked = !p.locked),
          trailing: Toggle(value: prefs.locked, onChanged: (v) => set((p) => p.locked = v))),
      sep(),
      row('favorite', favorite ? icx(Ix.starFilled) : ico(Ico.star), favorite ? 'Remove from Favorites' : 'Add to Favorites',
          () => onAction(PageMenuAction.favorite)),
      if (meta != null && !meta.missing && meta.parent == null)
        row('colour', icx(Ix.color), 'Colour', () => setState(() => _colour = true),
            trailing: Row(mainAxisSize: MainAxisSize.min, children: [
              SpaceDot(meta.color), const SizedBox(width: 6), ico(Ico.chevronRight),
            ])),
      row('copy-link', ico(Ico.link), 'Copy link', () => onAction(PageMenuAction.copyLink), shortcut: shortcutHint('Ctrl+L')),
      if (widget.canShare) row('share', icx(Ix.linkPage), shareMenuLabel(_shareCount), () => onAction(PageMenuAction.share)),
      row('duplicate', ico(Ico.duplicate), 'Duplicate', () => onAction(PageMenuAction.duplicate), shortcut: shortcutHint('Ctrl+D')),
      row('move-to', icx(Ix.move), 'Move to', () => onAction(PageMenuAction.moveTo)),
      if (widget.canAskAi) row('ask-ai', AiMark(size: iconSize), 'Ask AI', () => onAction(PageMenuAction.askAi)),
      sep(),
      row('undo', ico(Ico.undo), 'Undo', canUndo ? () => onAction(PageMenuAction.undo) : null, shortcut: shortcutHint('Ctrl+Z')),
      row('history', icx(Ix.history), 'Page history', () => onAction(PageMenuAction.history)),
      row('trash', icx(Ix.restore), 'Show deleted pages', () => onAction(PageMenuAction.trash)),
      row('delete', ico(Ico.trash, danger: true), 'Delete', () => onAction(PageMenuAction.delete), danger: true),
      sep(),
      row('import', icx(Ix.import), 'Import', () => onAction(PageMenuAction.import)),
      row('export', icx(Ix.export), 'Export', () => onAction(PageMenuAction.export)),
      sep(),
      row('feedback', icx(Ix.feedback), 'Send feedback', () => onAction(PageMenuAction.feedback), shortcut: shortcutHint('Ctrl+Shift+F')),
    ]);
  }
}
