// View settings (the sliders button of the toolbar): the view's name, the
// layout picker, property visibility and order, filter, sort, group, the
// layout's own options, where pages open, and duplicate or delete. A
// popover on desktop, a sheet on mobile. Also the layout picker used to add
// a view, and the active tab's menu.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../registry/property_registry.dart';
import '../../registry/view_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import '../../ui/database/config.dart';
import 'agenda_settings.dart';
import 'controller.dart';
import '../../ui/database/db_icons.dart';
import 'filter_editor.dart';
import 'property_menu.dart';
import '../../ui/database/values.dart';
import '../../ui/database/widgets.dart';

/// The icon of layout [k].
DbIco layoutIcon(ViewKind k) => viewRegistry.of(k)?.icon ?? switch (k) {
      ViewKind.table => DbIco.table,
      ViewKind.list => DbIco.list,
      ViewKind.board => DbIco.board,
      ViewKind.calendar => DbIco.calendar,
      ViewKind.gallery => DbIco.gallery,
      ViewKind.timeline => DbIco.timeline,
    };

const _layoutOrder = [ViewKind.table, ViewKind.board, ViewKind.timeline, ViewKind.calendar, ViewKind.list, ViewKind.gallery];

const _layoutHelp = {
  ViewKind.table: 'Table with types to store and view any kind of structured data',
  ViewKind.board: 'Board grouped by any select or status property',
  ViewKind.timeline: 'Timeline of pages with a date range',
  ViewKind.calendar: 'Calendar of pages by a date property',
  ViewKind.list: 'Minimal list of pages',
  ViewKind.gallery: 'Grid of cards with previews',
};

/// Makes sure [store] has what layout [k] needs (a date property for a
/// calendar or timeline, a groupable one for a board), adding one
/// when there is none. Returns the property key to use.
Future<String?> _ensureLayoutProp(CollectionStore store, ViewKind k) async {
  if (k == ViewKind.calendar || k == ViewKind.timeline) {
    return store.firstDateKey() ?? await store.addProperty(PropKind.date, name: 'Date');
  }
  if (k == ViewKind.board) {
    return store.firstGroupKey() ?? await store.addProperty(PropKind.status, name: 'Status');
  }
  return null;
}

/// Adds a view of layout [k] and shows it.
Future<void> addViewOf(CollectionStore store, ViewKind k) async {
  await _ensureLayoutProp(store, k);
  await store.addView(k);
}

/// Switches view [v] to layout [k].
Future<void> changeLayout(CollectionStore store, ViewDef v, ViewKind k) async {
  if (v.kind == k) return;
  final key = await _ensureLayoutProp(store, k);
  final cur = store.views.firstWhere((x) => x.id == v.id, orElse: () => v);
  final defaultName = cur.name == viewKindLabel(cur.kind) || cur.name == viewLabel(cur.kind);
  await store.updateView(cur.copyWith(
    kind: k,
    name: defaultName ? viewLabel(k) : null,
    groupBy: k == ViewKind.board ? (cur.groupBy ?? key) : null,
    dateProp: (k == ViewKind.calendar || k == ViewKind.timeline) ? (cur.dateProp ?? key) : null,
  ));
}

/// The 3x2 grid of layouts.
class LayoutGrid extends StatelessWidget {
  const LayoutGrid({super.key, required this.current, required this.onPick});
  final ViewKind? current;
  final ValueChanged<ViewKind> onPick;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget tile(ViewKind k) {
      final sel = k == current;
      final fg = sel ? c.accent : c.textSecondary;
      return Expanded(
        child: Padding(
          padding: const EdgeInsets.all(3),
          child: Hover(
            key: ValueKey('layout-${k.name}'),
            onTap: () => onPick(k),
            radius: Radii.md,
            child: Container(
              height: 62,
              decoration: BoxDecoration(
                border: Border.all(color: sel ? c.accent : c.border, width: sel ? 1.5 : 1),
                borderRadius: BorderRadius.circular(Radii.md),
              ),
              child: Column(mainAxisAlignment: MainAxisAlignment.center, children: [
                DbIcon(layoutIcon(k), size: 20, color: fg),
                const SizedBox(height: 6),
                Text(viewLabel(k), style: ts(context.type.uiSmall, fg, weight: sel ? 600 : 400)),
              ]),
            ),
          ),
        ),
      );
    }

    return Column(mainAxisSize: MainAxisSize.min, children: [
      Row(children: [for (final k in _layoutOrder.take(3)) tile(k)]),
      Row(children: [for (final k in _layoutOrder.skip(3)) tile(k)]),
    ]);
  }
}

/// "+" next to the tabs: picks a layout and adds a view.
Future<void> showAddView(BuildContext anchor, CollectionStore store) async {
  final k = await showPanel<ViewKind>(
    anchor,
    globalRectOf(anchor),
    (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      const MenuHeader('Add a view'),
      LayoutGrid(current: null, onPick: (k) => Navigator.of(ctx).pop(k)),
    ]),
    width: 300,
  );
  if (k != null) await addViewOf(store, k);
}

/// The menu of the active tab: rename, edit view, duplicate, delete.
Future<void> showViewMenu(BuildContext anchor, DbController ctl, ViewDef v) {
  final store = ctl.store;
  final name = TextEditingController(text: v.name);
  void rename() {
    final n = name.text.trim();
    final cur = store.views.where((x) => x.id == v.id).firstOrNull;
    if (cur != null && n.isNotEmpty && n != cur.name) store.updateView(cur.copyWith(name: n));
  }

  return showPanel<void>(
    anchor,
    globalRectOf(anchor),
    (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Padding(
        padding: const EdgeInsets.only(bottom: 4),
        child: MenuField(
          key: const ValueKey('view-name'),
          controller: name,
          leading: DbIcon(layoutIcon(v.kind), size: 16, color: ctx.colors.iconSecondary),
          onSubmitted: (_) => Navigator.of(ctx).pop(),
        ),
      ),
      dbItem(ctx, 'Edit view', DbIco.sliders, key: const ValueKey('view-edit'), onTap: () {
        Navigator.of(ctx).pop();
        showViewSettings(anchor, ctl);
      }),
      dbItem(ctx, 'Duplicate view', null, ico: Ico.duplicate, key: const ValueKey('view-duplicate'), onTap: () {
        Navigator.of(ctx).pop();
        store.duplicateView(v);
      }),
      if (store.views.length > 1)
        dbItem(ctx, 'Delete view', null, ico: Ico.trash, danger: true, key: const ValueKey('view-delete'), onTap: () {
          Navigator.of(ctx).pop();
          store.deleteView(v);
        }),
    ]),
  ).whenComplete(() {
    rename();
    name.dispose();
  });
}

/// The view settings panel.
Future<void> showViewSettings(BuildContext anchor, DbController ctl, {bool layout = false}) => showPanel<void>(
      anchor,
      globalRectOf(anchor),
      (ctx) => ViewSettings(ctl: ctl, anchor: anchor, startOnLayout: layout),
      width: 290,
    );

enum _Page { main, layout, properties, group, openIn, agenda }

class ViewSettings extends StatefulWidget {
  const ViewSettings({super.key, required this.ctl, required this.anchor, this.startOnLayout = false});
  final DbController ctl;
  final BuildContext anchor;
  final bool startOnLayout;

  @override
  State<ViewSettings> createState() => _ViewSettingsState();
}

class _ViewSettingsState extends State<ViewSettings> {
  late _Page page = widget.startOnLayout ? _Page.layout : _Page.main;
  late final _name = TextEditingController(text: widget.ctl.view?.name ?? '');
  AgendaSettings _agenda = const AgendaSettings(role: AgendaRole.auto);
  bool _agendaLoaded = false, _agendaSaving = false;
  int _agendaGeneration = 0;
  String? _agendaError;

  CollectionStore get store => widget.ctl.store;

  @override
  void initState() {
    super.initState();
    store.addListener(_changed);
    store.feed.addListener(_agendaChanged);
    _loadAgenda();
  }

  void _agendaChanged(ChangeBatch batch) {
    if (batch.reset || batch.changes.any((c) => c.page == store.collection)) _loadAgenda();
  }

  Future<void> _loadAgenda() async {
    final generation = ++_agendaGeneration;
    try {
      final settings = await store.vault.run((v) => v.agendaSettings(collection: store.collection));
      if (mounted && generation == _agendaGeneration && !_agendaSaving) {
        setState(() { _agenda = settings; _agendaLoaded = true; });
      }
    } catch (e) {
      if (mounted) setState(() => _agendaError = 'Could not load Today settings: $e');
    }
  }

  Future<void> _saveAgenda(AgendaSettings settings) async {
    if (_agendaSaving || !_agendaLoaded) return;
    ++_agendaGeneration;
    final previous = _agenda;
    setState(() { _agenda = settings; _agendaSaving = true; _agendaError = null; });
    try {
      await store.vault.run((v) => v.setAgendaSettings(collection: store.collection, settings: settings), writes: true);
    } catch (e) {
      if (mounted) setState(() { _agenda = previous; _agendaError = 'Could not save Today settings: $e'; });
    } finally {
      if (mounted) setState(() => _agendaSaving = false);
    }
  }

  void _changed() {
    if (mounted) setState(() {});
  }

  @override
  void dispose() {
    store.removeListener(_changed);
    store.feed.removeListener(_agendaChanged);
    final v = store.view;
    final n = _name.text.trim();
    if (v != null && n.isNotEmpty && n != v.name) store.updateView(v.copyWith(name: n));
    _name.dispose();
    super.dispose();
  }

  void _close() => Navigator.of(context).maybePop();

  @override
  Widget build(BuildContext context) {
    final v = store.view;
    if (v == null) return const SizedBox.shrink();
    return switch (page) {
      _Page.main => _main(context, v),
      _Page.layout => _layout(context, v),
      _Page.properties => _properties(context, v),
      _Page.group => _group(context, v),
      _Page.openIn => _openIn(context, v),
      _Page.agenda => _agendaPage(context),
    };
  }

  Widget _col(List<Widget> children) =>
      Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: children);

  Widget _main(BuildContext context, ViewDef v) {
    final rules = Filters.count(v.filter);
    final group = store.groupDef;
    return _col([
      Padding(
        padding: const EdgeInsets.fromLTRB(2, 2, 2, 6),
        child: Text('View settings', style: ts(context.type.ui, context.colors.textPrimary, weight: 600)),
      ),
      Padding(
        padding: const EdgeInsets.only(bottom: 4),
        child: MenuField(
          key: const ValueKey('settings-name'),
          controller: _name,
          leading: DbIcon(layoutIcon(v.kind), size: 16, color: context.colors.iconSecondary),
        ),
      ),
      dbItem(context, 'Layout', layoutIcon(v.kind),
          key: const ValueKey('settings-layout'),
          trailing: menuValue(context, viewLabel(v.kind)),
          onTap: () => setState(() => page = _Page.layout)),
      const MenuSeparator(),
      dbItem(context, 'Properties', DbIco.eye,
          key: const ValueKey('settings-properties'),
          trailing: menuValue(context, '${v.visibleProps.length} shown'),
          onTap: () => setState(() => page = _Page.properties)),
      dbItem(context, 'Filter', DbIco.filter,
          key: const ValueKey('settings-filter'),
          trailing: menuValue(context, rules == 0 ? 'None' : '$rules ${rules == 1 ? 'rule' : 'rules'}'),
          onTap: () {
        final a = widget.anchor;
        _close();
        filterButton(a, store);
      }),
      dbItem(context, 'Sort', DbIco.sort,
          key: const ValueKey('settings-sort'),
          trailing: menuValue(context, v.sorts.isEmpty ? 'None' : '${v.sorts.length} ${v.sorts.length == 1 ? 'sort' : 'sorts'}'),
          onTap: () {
        final a = widget.anchor;
        _close();
        sortButton(a, store);
      }),
      if (v.kind == ViewKind.board)
        dbItem(context, 'Group', DbIco.group,
            key: const ValueKey('settings-group'),
            trailing: menuValue(context, group?.name ?? 'None'),
            onTap: () => setState(() => page = _Page.group)),
      ..._layoutOptions(context, v),
      dbItem(context, 'Show in Today', DbIco.calendar,
          key: const ValueKey('settings-agenda'),
          trailing: menuValue(context, _agendaLoaded ? agendaSettingsLabel(store.schema, _agenda) : '…'),
          onTap: () => setState(() => page = _Page.agenda)),
      // Desktop opens every row as a sheet; the setting stays in the config.
      if (context.mobile)
        dbItem(context, 'Open pages in', DbIco.peekSide,
          key: const ValueKey('settings-open-in'),
          trailing: menuValue(context, _openLabel(v.cfg.openIn(v.kind))),
          onTap: () => setState(() => page = _Page.openIn)),
      const MenuSeparator(),
      dbItem(context, 'Duplicate view', null, ico: Ico.duplicate, key: const ValueKey('settings-duplicate'), onTap: () {
        _close();
        store.duplicateView(v);
      }),
      if (store.views.length > 1)
        dbItem(context, 'Delete view', null, ico: Ico.trash, danger: true, key: const ValueKey('settings-delete'), onTap: () {
          _close();
          store.deleteView(v);
        }),
    ]);
  }

  List<Widget> _layoutOptions(BuildContext context, ViewDef v) {
    final t = viewRegistry.of(v.kind);
    final opts = t?.options?.call(context, ViewArgs(store: store, view: v, ctl: widget.ctl, inline: false, gutter: 0));
    if (opts == null || opts.isEmpty) return const [];
    return [
      const MenuSeparator(),
      for (final o in opts)
        Builder(
          builder: (b) => dbItem(context, o.label, o.icon,
              key: ValueKey('opt-${o.label}'), trailing: o.value, onTap: () => o.onTap(b, globalRectOf(b))),
        ),
    ];
  }

  static String _openLabel(OpenIn o) => switch (o) {
        OpenIn.sidePeek => 'Side peek',
        OpenIn.centerPeek => 'Center peek',
        OpenIn.fullPage => 'Full page',
      };

  Widget _layout(BuildContext context, ViewDef v) => _col([
        menuBack(context, 'Layout', () => setState(() => page = _Page.main)),
        LayoutGrid(current: v.kind, onPick: (k) => changeLayout(store, v, k)),
        Padding(
          padding: const EdgeInsets.fromLTRB(4, 8, 4, 4),
          child: Text(_layoutHelp[v.kind] ?? '', style: ts(context.type.uiSmall, context.colors.textTertiary)),
        ),
        ..._layoutOptions(context, v),
        dbItem(context, 'Open pages in', null,
            trailing: menuValue(context, _openLabel(v.cfg.openIn(v.kind))), onTap: () => setState(() => page = _Page.openIn)),
      ]);

  Widget _properties(BuildContext context, ViewDef v) {
    final c = context.colors;
    final shown = store.visibleProps(v);
    final hidden = store.hiddenProps(v);
    Widget row(PropDef? d, bool visible, {int? index}) {
      final item = dbItem(
        context,
        d?.name ?? 'Name',
        propIcon(d),
        key: ValueKey('vis-prop-${d?.name ?? 'Name'}'),
        trailing: d == null
            ? null
            : DbIconBtn(visible ? DbIco.eye : DbIco.eyeOff,
                key: ValueKey('toggle-${d.name}'),
                size: 24,
                color: visible ? c.iconSecondary : c.iconTertiary,
                label: visible ? 'Hide' : 'Show',
                onTap: () => store.updateView(v.copyWith(
                    visibleProps: visible ? ([...v.visibleProps]..remove(d.key)) : [...v.visibleProps, d.key]))),
        onTap: null,
      );
      if (index == null) return item;
      return ReorderableDragStartListener(key: ValueKey('drag-${d!.key}'), index: index, child: item);
    }

    return _col([
      menuBack(context, 'Properties', () => setState(() => page = _Page.main)),
      row(null, true),
      if (shown.isNotEmpty)
        ConstrainedBox(
          constraints: const BoxConstraints(maxHeight: 320),
          child: ReorderableList(
            shrinkWrap: true,
            itemCount: shown.length,
            onReorderItem: (from, to) {
              final keys = [for (final d in shown) d.key];
              final k = keys.removeAt(from);
              keys.insert(to, k);
              store.updateView(v.copyWith(visibleProps: keys));
            },
            itemBuilder: (_, i) => KeyedSubtree(key: ValueKey('vp-${shown[i].key}'), child: row(shown[i], true, index: i)),
          ),
        ),
      if (hidden.isNotEmpty) ...[
        const MenuSeparator(),
        const MenuHeader('Hidden'),
        for (final d in hidden) row(d, false),
      ],
      const MenuSeparator(),
      Builder(
        builder: (b) => MenuItem(
          key: const ValueKey('settings-new-prop'),
          label: 'New property',
          icon: Ico.plus,
          onTap: () async {
            final kind = await pickPropertyType(b, globalRectOf(b));
            if (kind != null) await store.addProperty(kind, inView: v);
          },
        ),
      ),
    ]);
  }

  Widget _group(BuildContext context, ViewDef v) {
    final c = context.colors;
    final candidates = [for (final d in store.schema) if (propertyRegistry.of(d.kind)?.groupable ?? false) d];
    final cfg = v.cfg;
    return _col([
      menuBack(context, 'Group', () => setState(() => page = _Page.main)),
      const MenuHeader('Group by'),
      for (final d in candidates)
        dbItem(context, d.name, propIcon(d),
            key: ValueKey('group-by-${d.name}'),
            trailing: v.groupBy == d.key ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
            onTap: () => store.updateView(v.copyWith(groupBy: d.key))),
      if (store.groupDef != null) ...[
        const MenuSeparator(),
        dbItem(context, 'Hide empty groups', null,
            key: const ValueKey('group-hide-empty'),
            trailing: Toggle(on: cfg.hideEmptyGroups),
            onTap: () => store.updateConfig(v, (c) => c.hideEmptyGroups = !c.hideEmptyGroups)),
        dbItem(context, 'Color columns', null,
            key: const ValueKey('group-color'),
            trailing: Toggle(on: cfg.colorColumns),
            onTap: () => store.updateConfig(v, (c) => c.colorColumns = !c.colorColumns)),
        const MenuSeparator(),
        const MenuHeader('Groups'),
        for (final g in store.groups)
          dbItem(context, g.group.label.isEmpty ? 'No ${store.groupDef!.name}' : g.group.label, null,
              key: ValueKey('group-vis-${g.key}'),
              trailing: DbIcon(cfg.hiddenGroups.contains(g.key) ? DbIco.eyeOff : DbIco.eye,
                  size: 16, color: cfg.hiddenGroups.contains(g.key) ? c.iconTertiary : c.iconSecondary),
              onTap: () => store.updateConfig(v, (c) {
                    final h = c.hiddenGroups;
                    h.contains(g.key) ? h.remove(g.key) : h.add(g.key);
                    c.hiddenGroups = h;
                  })),
      ],
    ]);
  }

  Widget _agendaPage(BuildContext context) {
    final c = context.colors;
    final dates = store.schema.where((p) => p.kind == PropKind.date);
    return _col([
      menuBack(context, 'Show in Today', () => setState(() => page = _Page.main)),
      for (final role in AgendaRole.values)
        dbItem(context, role == AgendaRole.auto ? agendaSettingsLabel(store.schema, const AgendaSettings(role: AgendaRole.auto)) : agendaRoleLabel(role), null,
          key: ValueKey('agenda-role-${role.name}'),
          trailing: _agenda.role == role ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
          onTap: !_agendaLoaded || _agendaSaving ? null : () => _saveAgenda(AgendaSettings(role: role, dateProp: _agenda.dateProp))),
      const MenuSeparator(),
      const MenuHeader('Date property'),
      dbItem(context, 'First date property', DbIco.date,
          key: const ValueKey('agenda-date-first'),
          trailing: _agenda.dateProp == null || !dates.any((p) => p.key == _agenda.dateProp)
              ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
          onTap: !_agendaLoaded || _agendaSaving ? null : () => _saveAgenda(AgendaSettings(role: _agenda.role))),
      for (final prop in dates)
        dbItem(context, prop.name, DbIco.date,
          key: ValueKey('agenda-date-${prop.key}'),
          trailing: _agenda.dateProp == prop.key ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
          onTap: !_agendaLoaded || _agendaSaving ? null : () => _saveAgenda(AgendaSettings(role: _agenda.role, dateProp: prop.key))),
      if (_agendaError != null) Padding(padding: const EdgeInsets.all(8), child: Text(_agendaError!, style: ts(context.type.uiSmall, c.inlineCodeText))),
    ]);
  }

  Widget _openIn(BuildContext context, ViewDef v) {
    final c = context.colors;
    final cur = v.cfg.openIn(v.kind);
    return _col([
      menuBack(context, 'Open pages in', () => setState(() => page = _Page.main)),
      for (final (o, icon) in const [
        (OpenIn.sidePeek, DbIco.peekSide),
        (OpenIn.centerPeek, DbIco.peekCenter),
        (OpenIn.fullPage, DbIco.fullPage),
      ])
        dbItem(context, _openLabel(o), icon,
            key: ValueKey('open-in-${o.name}'),
            trailing: o == cur ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
            onTap: () => store.updateConfig(v, (c) => c.openInValue = o)),
    ]);
  }
}
