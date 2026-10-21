// "Move to": pick a new parent page (or the top level) for a page. A page
// search (the editor's searchPages) without the page itself and the pages
// below it, plus "Pages (top level)". The page goes to the end of its new
// parent's children. A popover on desktop, a sheet on mobile.

import '../core/debounce.dart';

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../editor/block_pickers.dart' show PageHit, searchPages;
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../state/tree_store.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';

/// Where "Move to" was asked to put a page: [parent] null = top level.
class MoveTarget {
  const MoveTarget(this.parent);
  final String? parent;
}

/// Asks for a new place for page [pageId] and moves it there.
Future<void> showMoveTo(BuildContext context, String pageId, {Rect? anchor}) async {
  final tree = context.read<TreeStore>();
  Widget body(BuildContext ctx) => MoveToList(
        key: const ValueKey('move-to'),
        pageId: pageId,
        onPick: (t) => Navigator.of(ctx).pop(t),
      );
  final MoveTarget? t;
  if (context.mobile || anchor == null) {
    t = await showSheet<MoveTarget>(context, (ctx) => SheetSurface(children: [body(ctx)]));
  } else {
    t = await showPopover<MoveTarget>(context, anchor, (ctx) => MenuSurface(width: 320, child: body(ctx)));
  }
  if (t == null) return;
  try {
    await tree.moveToEnd(pageId, t.parent);
  } catch (e) {
    debugPrint('move_page failed: $e');
  }
}

class MoveToList extends StatefulWidget {
  const MoveToList({super.key, required this.pageId, required this.onPick});
  final String pageId;
  final ValueChanged<MoveTarget> onPick;

  @override
  State<MoveToList> createState() => _MoveToListState();
}

class _MoveToListState extends State<MoveToList> {
  final _ctl = TextEditingController();
  final _focus = FocusNode();
  List<PageHit> _hits = const [];
  Set<String>? _excluded;
  Timer? _debounce;
  int _gen = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) => _start());
  }

  Future<void> _start() async {
    if (!mounted) return;
    final tree = context.read<TreeStore>();
    try {
      final d = await tree.descendants(widget.pageId);
      _excluded = {widget.pageId, ...d};
    } catch (_) {
      _excluded = {widget.pageId};
    }
    await _search();
  }

  @override
  void dispose() {
    _debounce?.cancel();
    _ctl.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _search() async {
    final g = ++_gen;
    if (!mounted) return;
    final hits = await searchPages(context, _ctl.text, limit: 12);
    if (!mounted || g != _gen) return;
    final ex = _excluded ?? {widget.pageId};
    setState(() => _hits = [for (final h in hits) if (!ex.contains(h.id)) h]);
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final m = context.mobile;
    final empty = _ctl.text.trim().isEmpty;
    return Padding(
      padding: EdgeInsets.all(m ? 0 : 6),
      child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        Padding(
          padding: EdgeInsets.fromLTRB(m ? Layout.gutter : 2, 2, m ? Layout.gutter : 2, 6),
          child: InputBox(
            height: m ? 40 : 30,
            child: PlainField(
              key: const ValueKey('move-to-field'),
              controller: _ctl,
              focusNode: _focus,
              autofocus: !m,
              style: ts(ty.ui, c.textPrimary, weight: 400),
              placeholder: 'Move page to…',
              onChanged: (_) {
                _debounce?.cancel();
                _debounce = Timer(Debounce.testDelay(const Duration(milliseconds: 150)), _search);
              },
              onSubmitted: (_) {
                if (_hits.isNotEmpty) widget.onPick(MoveTarget(_hits.first.id));
              },
            ),
          ),
        ),
        if (empty) ...[
          m ? const SheetHeader('Pages') : const MenuHeader('Pages'),
          _item(
            key: const ValueKey('move-to-root'),
            label: 'Pages (top level)',
            leading: IconX(Ix.subPage, size: m ? 22 : 20, color: c.iconSecondary),
            onTap: () => widget.onPick(const MoveTarget(null)),
          ),
        ],
        m
            ? SheetHeader(empty ? 'Suggested' : (_hits.isEmpty ? 'No results' : 'Pages'))
            : MenuHeader(empty ? 'Suggested' : (_hits.isEmpty ? 'No results' : 'Pages')),
        for (final h in _hits)
          _item(
            key: ValueKey('move-pick-${h.id}'),
            label: displayTitle(h.title, firstLine: context.watch<PagesStore>().meta(h.id)?.firstLine ?? ''),
            leading: PageIcon(icon: h.icon, size: m ? 22 : 20),
            onTap: () => widget.onPick(MoveTarget(h.id)),
          ),
      ]),
    );
  }

  Widget _item({required Key key, required String label, required Widget leading, required VoidCallback onTap}) =>
      context.mobile
          ? SheetItem(key: key, label: label, leading: leading, onTap: onTap)
          : MenuItem(key: key, label: label, leading: leading, onTap: onTap);
}
