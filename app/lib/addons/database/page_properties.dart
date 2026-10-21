// The top of a page: the property rows of a database row (name column with
// the type icon, the value, "+ Add a property", hidden ones behind "N more
// properties"), and the switch between the page editor and a full-page
// database.
//
// The page editor reaches this file through two insertion points in
// editor/page_screen.dart: [PageProperties] under the title and
// the database add-on API.

import '../../core/debounce.dart';

import 'dart:async';

import 'package:flutter/widgets.dart';

import '../../core/change_feed.dart';
import '../../core/models.dart';
import '../../registry/property_registry.dart';
import '../../state/app_state.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import '../../services/legacy_host.dart';
import '../../ui/database/config.dart';
import 'controller.dart';
import '../../ui/database/db_icons.dart';
import 'property_menu.dart';
import '../../ui/database/values.dart';

class PageProperties extends StatefulWidget {
  const PageProperties({
    super.key,
    required this.pageId,
    required this.parent,
    required this.left,
    required this.width,
  });
  final String pageId;
  final String? parent;

  /// The text column (the editor's geometry).
  final double left, width;

  @override
  State<PageProperties> createState() => _PagePropertiesState();
}

class _PagePropertiesState extends State<PageProperties> {
  CollectionStore? _store;
  DbController? _ctl;
  PageRow? _row;
  bool _showHidden = false;
  ChangeFeed? _feed;
  Timer? _reload;
  int _pending = 0;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_store == null && widget.parent != null) _open(widget.parent!);
  }

  @override
  void didUpdateWidget(PageProperties old) {
    super.didUpdateWidget(old);
    if (old.parent != widget.parent || old.pageId != widget.pageId) {
      _close();
      if (widget.parent != null) _open(widget.parent!);
    }
  }

  void _open(String parent) {
    final app = context.app;
    final s = CollectionStore(app.vault, app.feed, parent, withRows: false);
    s.addListener(_changed);
    _store = s;
    _ctl = DbController(s);
    _feed = app.feed..addListener(_onFeed);
    s.load();
    _loadRow();
  }

  void _close() {
    _store?.removeListener(_changed);
    _store?.dispose();
    _store = null;
    _ctl = null;
    _row = null;
    _reload?.cancel();
    _feed?.removeListener(_onFeed);
    _feed = null;
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  void _onFeed(ChangeBatch b) {
    if (!b.touches(widget.pageId) || _pending > 0) return;
    _reload?.cancel();
    _reload = Timer(
      Debounce.testDelay(const Duration(milliseconds: 150)),
      _loadRow,
    );
  }

  Future<void> _loadRow() async {
    final p = await context.app.vault.run((v) => v.loadPage(id: widget.pageId));
    if (!mounted) return;
    setState(() => _row = CollectionStore.rowOf(p));
  }

  @override
  void dispose() {
    _close();
    super.dispose();
  }

  Future<void> _set(PropDef d, PropValue v) async {
    final r = _row;
    if (r == null) return;
    setState(() => _row = r.withValue(d.key, v));
    _pending++;
    try {
      await _store!.setValue(widget.pageId, d.key, v);
    } finally {
      _pending--;
    }
  }

  Future<void> _edit(BuildContext cell, PropDef d) async {
    final r = _row;
    final t = propertyRegistry.of(d.kind);
    if (r == null || t == null) return;

    final v = r.valueOf(d);
    if (t.tap != null) return _set(d, t.tap!(v));

    final e = t.edit;
    if (e == null) return;
    await e(
      EditArgs(
        context: cell,
        def: d,
        value: v,
        anchor: globalRectOf(cell),
        onChanged: (nv) => _set(d, nv),
        saveDef: _store!.updateProperty,
        collection: _store!.collection,
      ),
    );
  }

  Future<void> _add(BuildContext anchor) async {
    final kind = await pickPropertyType(anchor, globalRectOf(anchor));
    if (kind == null || _store == null) return;

    final key = await _store!.addProperty(kind, inView: null);
    final d = _store?.def(key);
    if (d != null && anchor.mounted) {
      await showPropertyMenu(anchor, _ctl!, d, editing: true, inPage: true);
    }
  }

  @override
  Widget build(BuildContext context) => withTemplates(
    pageId: widget.pageId,
    parent: widget.parent,
    left: widget.left,
    width: widget.width,
    child: _build(context),
  );

  Widget _build(BuildContext context) {
    final s = _store;
    final mobile = context.mobile;
    final pad = EdgeInsets.only(left: widget.left, bottom: 8);
    if (s == null || !s.loaded || !s.isDatabase) {
      return const SizedBox.shrink();
    }
    final r = _row;
    if (r == null) {
      return SizedBox(height: s.schema.length * (mobile ? 38.0 : 34.0));
    }
    final shown = <PropDef>[], hidden = <PropDef>[];
    for (final d in s.schema) {
      final vis = d.cfg.visibility;
      final empty = r.valueOf(d).isEmpty;
      if (vis == PropVisibility.hidden ||
          (vis == PropVisibility.hideEmpty && empty)) {
        hidden.add(d);
      } else {
        shown.add(d);
      }
    }
    final c = context.colors;
    return Padding(
      padding: pad,
      child: Align(
        alignment: Alignment.topLeft,
        child: SizedBox(
          width: widget.width,
          child: Column(
            key: const ValueKey('page-properties'),
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              for (final d in shown) _propRow(context, d, r),
              if (_showHidden)
                for (final d in hidden) _propRow(context, d, r),
              if (hidden.isNotEmpty)
                _footRow(
                  context,
                  key: const ValueKey('props-more'),
                  icon: Icon20(
                    _showHidden ? Ico.chevronDown : Ico.chevronRight,
                    size: 14,
                    color: c.iconTertiary,
                  ),
                  label: _showHidden
                      ? 'Hide ${hidden.length} ${hidden.length == 1 ? 'property' : 'properties'}'
                      : '${hidden.length} more ${hidden.length == 1 ? 'property' : 'properties'}',
                  onTap: (_) => setState(() => _showHidden = !_showHidden),
                ),
              _footRow(
                context,
                key: const ValueKey('props-add'),
                icon: Icon20(Ico.plus, size: 16, color: c.iconTertiary),
                label: 'Add a property',
                onTap: _add,
              ),
              const SizedBox(height: 8),
              HLine(color: c.divider),
              const SizedBox(height: 4),
            ],
          ),
        ),
      ),
    );
  }

  Widget _propRow(BuildContext context, PropDef d, PageRow r) {
    final c = context.colors;
    final mobile = context.mobile;
    final t = propertyRegistry.of(d.kind);
    final v = r.valueOf(d);
    const labelW = 140.0;
    final labelStyle = tsRaw(14, 20, 400, c.textSecondary);
    return Padding(
      padding: const EdgeInsets.only(bottom: 4),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: mobile ? 34 : 30),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: labelW,
              child: Builder(
                builder: (b) => Hover(
                  key: ValueKey('prop-label-${d.name}'),
                  onTap: () => showPropertyMenu(b, _ctl!, d, inPage: true),
                  radius: Radii.sm,
                  padding: const EdgeInsets.symmetric(horizontal: 6),
                  child: SizedBox(
                    height: mobile ? 34 : 30,
                    child: Row(
                      children: [
                        DbIcon(
                          propIcon(d),
                          size: mobile ? 18 : 16,
                          color: c.textSecondary,
                        ),
                        SizedBox(width: mobile ? 10 : 8),
                        Expanded(
                          child: Text(
                            d.name,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: labelStyle,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
            const SizedBox(width: 4),
            Expanded(
              child: Builder(
                builder: (b) => Hover(
                  key: ValueKey('prop-value-${d.name}'),
                  onTap: t?.edit == null && t?.tap == null
                      ? null
                      : () => _edit(b, d),
                  radius: Radii.sm,
                  padding: const EdgeInsets.symmetric(
                    horizontal: 6,
                    vertical: 5,
                  ),
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: t == null
                        ? const SizedBox.shrink()
                        : t.cell(
                            CellArgs(
                              context: b,
                              def: d,
                              value: v,
                              mode: CellMode.page,
                              wrap: true,
                            ),
                          ),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Widget _footRow(
    BuildContext context, {
    required Key key,
    required Widget icon,
    required String label,
    required void Function(BuildContext) onTap,
  }) {
    final c = context.colors;
    return Builder(
      builder: (b) => Hover(
        key: key,
        onTap: () => onTap(b),
        radius: Radii.sm,
        padding: const EdgeInsets.symmetric(horizontal: 6),
        child: SizedBox(
          height: context.mobile ? 34 : 30,
          child: Row(
            children: [
              SizedBox(width: 16, child: Center(child: icon)),
              const SizedBox(width: 8),
              Text(
                label,
                style: ts(
                  context.mobile
                      ? const TypeSpec(15, 20, 400)
                      : const TypeSpec(14, 20, 400),
                  c.textTertiary,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
