// The property menu (click a column header or a property row's name):
// rename, "Edit property" (type, format, options), filter, sort, group,
// freeze, hide, wrap, insert left or right, duplicate, delete. Also the
// property type picker ("Basic" and "Advanced" types, searchable).

import 'package:flutter/widgets.dart';

import '../../core/models.dart' hide StatusGroup;
import 'properties/date.dart';
import 'properties/formula.dart';
import 'properties/number.dart';
import 'properties/rollup.dart';
import '../../registry/property_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import '../../ui/database/config.dart';
import 'controller.dart';
import '../../ui/database/db_icons.dart';
import '../../ui/database/db_theme.dart';
import 'editors/option_picker.dart';
import 'filter_editor.dart';
import '../../ui/database/values.dart';
import '../../ui/database/widgets.dart';

/// The icon of property [d] (the title when null).
DbIco propIcon(PropDef? d) => d == null ? DbIco.title : (propertyRegistry.of(d.kind)?.icon ?? DbIco.text);

String propTypeLabel(PropKind k) => propertyRegistry.of(k)?.label ?? CollectionStore.defaultPropName(k);

/// Shows the type picker under [rect]; completes with the chosen type.
Future<PropKind?> pickPropertyType(BuildContext context, Rect rect, {PropKind? current}) =>
    showPanel<PropKind>(context, rect, (ctx) => TypePicker(current: current, onPick: (k) => Navigator.of(ctx).pop(k)), width: 290);

class TypePicker extends StatefulWidget {
  const TypePicker({super.key, required this.onPick, this.current});
  final ValueChanged<PropKind> onPick;
  final PropKind? current;

  @override
  State<TypePicker> createState() => _TypePickerState();
}

class _TypePickerState extends State<TypePicker> {
  String q = '';

  @override
  Widget build(BuildContext context) {
    final types = [for (final t in propertyRegistry.all) if (q.isEmpty || t.label.toLowerCase().contains(q)) t];
    Widget item(PropertyType t) => dbItem(context, t.label, t.icon,
        key: ValueKey('type-${t.kind.name}'),
        selected: t.kind == widget.current,
        trailing: t.kind == widget.current ? Icon20(Ico.check, size: 16, color: context.colors.iconPrimary) : null,
        onTap: () => widget.onPick(t.kind));
    final basic = [for (final t in types) if (!t.advanced) item(t)];
    final adv = [for (final t in types) if (t.advanced) item(t)];
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      MenuField(
        key: const ValueKey('type-search'),
        placeholder: 'Search for a property type…',
        autofocus: !context.mobile,
        onChanged: (s) => setState(() => q = s.trim().toLowerCase()),
      ),
      const SizedBox(height: 4),
      Flexible(
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            if (basic.isNotEmpty) ...[const MenuHeader('Basic'), ...basic],
            if (adv.isNotEmpty) ...[const MenuHeader('Advanced'), ...adv],
          ]),
        ),
      ),
    ]);
  }
}

/// Opens the menu of property [def] (null: the title) from [anchor].
/// [inPage]: opened from a row page's property list (no view options;
/// the property's visibility on pages instead).
Future<void> showPropertyMenu(BuildContext anchor, DbController ctl, PropDef? def, {bool editing = false, bool inPage = false}) =>
    showPanel<void>(anchor, globalRectOf(anchor),
        (ctx) => PropertyMenu(ctl: ctl, def: def, editing: editing, anchor: anchor, inPage: inPage),
        width: 290);

enum _Page { main, edit, type, numberFormat, dateFormat, statusGroup }

class PropertyMenu extends StatefulWidget {
  const PropertyMenu(
      {super.key, required this.ctl, required this.def, this.editing = false, required this.anchor, this.inPage = false});
  final DbController ctl;
  final bool inPage;
  final PropDef? def;
  final bool editing;
  final BuildContext anchor;

  @override
  State<PropertyMenu> createState() => _PropertyMenuState();
}

class _PropertyMenuState extends State<PropertyMenu> {
  late PropDef? def = widget.def;
  late _Page page = widget.editing ? _Page.edit : _Page.main;
  late final _name = TextEditingController(text: widget.def?.name ?? 'Name');

  CollectionStore get store => widget.ctl.store;
  ViewDef? get view => store.view;

  @override
  void dispose() {
    _rename();
    _name.dispose();
    super.dispose();
  }

  void _rename() {
    final d = def;
    final n = _name.text.trim();
    if (d == null || n.isEmpty || n == d.name) return;
    def = d.copyWith(name: n);
    store.updateProperty(def!);
  }

  Future<void> _save(PropDef d) async {
    setState(() => def = d);
    final saved = await store.updateProperty(d);
    if (mounted) setState(() => def = saved);
  }

  void _close() => Navigator.of(context).maybePop();

  String get _key => def?.key ?? BuiltinKeys.title;

  Future<void> _sort(bool desc) async {
    final v = view;
    if (v == null) return;
    final sorts = [SortSpec(prop: _key, descending: desc), for (final s in v.sorts) if (s.prop != _key) s];
    _close();
    await store.updateView(v.copyWith(sorts: sorts));
  }

  Future<void> _hide() async {
    final v = view;
    if (v == null || def == null) return;
    _close();
    await store.updateView(v.copyWith(visibleProps: [...v.visibleProps]..remove(def!.key)));
  }

  Future<void> _insert(bool right) async {
    final v = view;
    if (v == null) return;
    final i = def == null ? 0 : v.visibleProps.indexOf(def!.key) + (right ? 1 : 0);
    final anchor = widget.anchor;
    _close();
    final kind = await pickPropertyType(anchor, globalRectOf(anchor));
    if (kind == null) return;
    await store.addProperty(kind, inView: v, index: i);
  }

  @override
  Widget build(BuildContext context) {
    return switch (page) {
      _Page.main => _main(context),
      _Page.edit => _edit(context),
      _Page.type => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          menuBack(context, 'Type', () => setState(() => page = _Page.edit)),
          Flexible(
            child: TypePicker(
              current: def?.kind,
              onPick: (k) async {
                final d = def;
                if (d == null) return;
                setState(() => page = _Page.edit);
                final saved = await store.changeType(d, k);
                if (mounted) setState(() => def = saved);
              },
            ),
          ),
        ]),
      _Page.numberFormat => _choices(context, 'Number format', numberFormats, def!.cfg.numberFormat, (id) {
          final c = def!.cfg..numberFormat = id;
          _save(def!.copyWith(config: c.encode()));
        }),
      _Page.dateFormat => _choices(context, 'Date format', dateFormats, def!.cfg.dateFormat, (id) {
          final c = def!.cfg..dateFormat = id;
          _save(def!.copyWith(config: c.encode()));
        }),
      _Page.statusGroup => const SizedBox.shrink(),
    };
  }

  Widget _choices(BuildContext context, String title, List<(String, String)> items, String current, ValueChanged<String> pick) =>
      Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        menuBack(context, title, () => setState(() => page = _Page.edit)),
        for (final (id, label) in items)
          MenuItem(
            key: ValueKey('choice-$id'),
            label: label,
            trailing: id == current ? Icon20(Ico.check, size: 16, color: context.colors.iconPrimary) : null,
            onTap: () {
              pick(id);
              setState(() => page = _Page.edit);
            },
          ),
      ]);

  Widget _nameRow(BuildContext context) => Row(children: [
        Container(
          width: 28,
          height: 28,
          decoration: BoxDecoration(border: Border.all(color: context.colors.borderTranslucent), borderRadius: BorderRadius.circular(Radii.md)),
          child: Center(child: DbIcon(propIcon(def), size: 16, color: context.colors.iconSecondary)),
        ),
        const SizedBox(width: 6),
        Expanded(
          child: def == null
              ? Text('Name', style: ts(context.type.ui, context.colors.textPrimary, weight: 400))
              : MenuField(key: const ValueKey('prop-name'), controller: _name, onSubmitted: (_) => _close()),
        ),
      ]);

  Widget _main(BuildContext context) {
    final d = def;
    final v = view;
    final type = d == null ? null : propertyRegistry.of(d.kind);
    final table = v?.kind == ViewKind.table;
    final board = v?.kind == ViewKind.board;
    final wrapped = v != null && (v.cfg.wrap || v.cfg.wrapped.contains(_key));
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      _nameRow(context),
      const SizedBox(height: 4),
      if (d != null)
        dbItem(context, 'Edit property', DbIco.sliders,
            key: const ValueKey('prop-edit'), onTap: () => setState(() => page = _Page.edit)),
      if (widget.inPage && d != null) ...[
        const MenuSeparator(),
        const MenuHeader('Property visibility'),
        for (final pv in PropVisibility.values)
          MenuItem(
            key: ValueKey('vis-${pv.name}'),
            label: pv.label,
            leading: DbIcon(pv == PropVisibility.hidden ? DbIco.eyeOff : DbIco.eye, size: 20, color: context.colors.iconPrimary),
            trailing: d.cfg.visibility == pv ? Icon20(Ico.check, size: 16, color: context.colors.iconPrimary) : null,
            onTap: () {
              final c = d.cfg..visibility = pv;
              _save(d.copyWith(config: c.encode()));
            },
          ),
      ],
      if (!widget.inPage) ...[
      const MenuSeparator(),
      dbItem(context, 'Filter', DbIco.filter, key: const ValueKey('prop-filter'), onTap: () {
        final a = widget.anchor;
        _close();
        addFilterFor(a, store, _key);
      }),
      dbItem(context, 'Sort ascending', DbIco.sort, key: const ValueKey('prop-sort-asc'), onTap: () => _sort(false)),
      dbItem(context, 'Sort descending', DbIco.sort, key: const ValueKey('prop-sort-desc'), onTap: () => _sort(true)),
      if (board && d != null && (type?.groupable ?? false))
        dbItem(context, 'Group', DbIco.group, key: const ValueKey('prop-group'), onTap: () {
          _close();
          store.updateView(v!.copyWith(groupBy: d.key));
        }),
      if (table && v != null) ...[
        dbItem(context, v.cfg.frozen > 0 && _frozenHere(v) ? 'Unfreeze columns' : 'Freeze up to column', DbIco.pin,
            key: const ValueKey('prop-freeze'), onTap: () {
          _close();
          store.updateConfig(v, (c) => c.frozen = _frozenHere(v) ? 0 : (d == null ? 0 : v.visibleProps.indexOf(d.key) + 1));
        }),
      ],
      if (d != null) dbItem(context, 'Hide', DbIco.eyeOff, key: const ValueKey('prop-hide'), onTap: _hide),
      if (table && v != null)
        dbItem(context, wrapped ? 'Unwrap text' : 'Wrap text', DbIco.wrap, key: const ValueKey('prop-wrap'), onTap: () {
          _close();
          store.updateConfig(v, (c) {
            final w = c.wrapped;
            wrapped ? w.remove(_key) : w.add(_key);
            if (c.wrap && wrapped) c.wrap = false;
            c.wrapped = w;
          });
        }),
      if (table) ...[
        if (d != null) dbItem(context, 'Insert left', DbIco.insertLeft, key: const ValueKey('prop-insert-left'), onTap: () => _insert(false)),
        dbItem(context, 'Insert right', DbIco.insertRight, key: const ValueKey('prop-insert-right'), onTap: () => _insert(true)),
      ],
      ],
      if (d != null) ...[
        const MenuSeparator(),
        dbItem(context, 'Duplicate property', null, ico: Ico.duplicate, key: const ValueKey('prop-duplicate'), onTap: () {
          _close();
          store.duplicateProperty(d, inView: widget.inPage ? null : v);
        }),
        dbItem(context, 'Delete property', null, ico: Ico.trash, danger: true, key: const ValueKey('prop-delete'), onTap: () {
          _close();
          store.deleteProperty(d.key);
        }),
      ],
    ]);
  }

  bool _frozenHere(ViewDef v) {
    final i = def == null ? 0 : v.visibleProps.indexOf(def!.key) + 1;
    return v.cfg.frozen == i && i > 0;
  }

  Widget _edit(BuildContext context) {
    final d = def;
    if (d == null) return _main(context);
    final c = context.colors;
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      menuBack(context, 'Edit property', () => setState(() => page = _Page.main)),
      _nameRow(context),
      const SizedBox(height: 4),
      Builder(
        builder: (b) => dbItem(context, 'Type', null,
            ico: Ico.turnInto,
            key: const ValueKey('prop-type'),
            trailing: menuValue(context, propTypeLabel(d.kind)),
            onTap: d.isTime ? null : () => setState(() => page = _Page.type)),
      ),
      if (d.kind == PropKind.number)
        dbItem(context, 'Number format', DbIco.number,
            key: const ValueKey('prop-number-format'),
            trailing: menuValue(context, numberFormats.firstWhere((f) => f.$1 == d.cfg.numberFormat, orElse: () => numberFormats.first).$2),
            onTap: () => setState(() => page = _Page.numberFormat)),
      if (d.kind == PropKind.formula) formulaSettings(context, store, d, _save),
      if (d.kind == PropKind.rollup) rollupSettings(context, store, d, _save),
      if (d.kind == PropKind.date || d.isTime)
        dbItem(context, 'Date format', DbIco.date,
            key: const ValueKey('prop-date-format'),
            trailing: menuValue(context, dateFormats.firstWhere((f) => f.$1 == d.cfg.dateFormat, orElse: () => dateFormats.first).$2),
            onTap: () => setState(() => page = _Page.dateFormat)),
      if (d.hasOptions) ...[
        const MenuSeparator(),
        Row(children: [
          const Expanded(child: MenuHeader('Options')),
          IconBtn(Ico.plus,
              key: const ValueKey('prop-add-option'),
              size: 22,
              glyph: 16,
              semanticLabel: 'Add an option',
              onTap: () => _save(d.copyWith(options: [
                    ...d.options,
                    SelectOption(id: '', name: _newOptionName(d), color: nextOptionColor(d.options)),
                  ]))),
        ]),
        if (d.kind == PropKind.status)
          for (final g in StatusGroup.values) ...[
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 4, 8, 2),
              child: Text(g.label, style: ts(context.type.uiSmall, c.textSecondary)),
            ),
            for (final o in d.options)
              if (statusGroupOf(d, o.id) == g) _optionRow(context, d, o),
          ]
        else
          ReorderableList(
            shrinkWrap: true,
            physics: const NeverScrollableScrollPhysics(),
            itemCount: d.options.length,
            onReorderItem: (from, to) {
              final list = [...d.options];
              final o = list.removeAt(from);
              list.insert(to.clamp(0, list.length), o);
              _save(d.copyWith(options: list));
            },
            itemBuilder: (context, i) => KeyedSubtree(
              key: ValueKey('edit-opt-${d.options[i].id}'),
              child: Row(children: [
                ReorderableDragStartListener(
                  index: i,
                  child: SizedBox(width: 18, height: 28, child: Icon20(Ico.grip, size: 14, color: c.iconTertiary)),
                ),
                Expanded(child: _optionRow(context, d, d.options[i])),
              ]),
            ),
          ),
      ],
      const MenuSeparator(),
      dbItem(context, 'Duplicate property', null, ico: Ico.duplicate, onTap: () {
        _close();
        store.duplicateProperty(d, inView: view);
      }),
      dbItem(context, 'Delete property', null, ico: Ico.trash, danger: true, key: const ValueKey('edit-delete'), onTap: () {
        _close();
        store.deleteProperty(d.key);
      }),
    ]);
  }

  static String _newOptionName(PropDef d) {
    var i = d.options.length + 1;
    while (d.options.any((o) => o.name == 'Option $i')) {
      i++;
    }
    return 'Option $i';
  }

  Widget _optionRow(BuildContext context, PropDef d, SelectOption o) => Builder(
        builder: (row) => Hover(
          key: ValueKey('edit-option-${o.name}'),
          onTap: () => editOptionOf(context, globalRectOf(row), d, o, (nd) async {
            await _save(nd);
            return def!;
          }),
          radius: Radii.md,
          padding: const EdgeInsets.symmetric(horizontal: 6),
          child: SizedBox(
            height: context.mobile ? 44 : 28,
            child: Row(children: [
              Expanded(
                child: Align(alignment: Alignment.centerLeft, child: Pill(o.name, color: o.color, status: d.kind == PropKind.status,
                  group: d.kind == PropKind.status
                      ? statusGroupOf(d, o.id)
                      : null)),
              ),
              if (d.kind == PropKind.status)
                Text(colorLabel(o.color), style: ts(context.type.uiSmall, context.colors.textTertiary, weight: 400)),
              Icon20(Ico.chevronRight, size: 14, color: context.colors.iconTertiary),
            ]),
          ),
        ),
      );
}
