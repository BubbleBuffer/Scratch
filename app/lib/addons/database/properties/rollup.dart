// Rollup: a value the core computes over the pages a relation links to:
// one of their properties, shown as is or aggregated (count, sum, average,
// min, max, percent checked, ...). Set up from the property menu: the
// relation, the related property and the calculation.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../editors/relation_picker.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';
import '../../../registry/property_registry.dart';
import '../../../state/collection_store.dart';
import '../../../ui/ui.dart';
import 'computed_values.dart';

/// Calculations in display order: (core function id, label).
const rollupFunctions = [
  ('show_original', 'Show original'),
  ('count', 'Count all'),
  ('count_values', 'Count values'),
  ('count_unique', 'Count unique values'),
  ('count_empty', 'Count empty'),
  ('count_not_empty', 'Count not empty'),
  ('percent_empty', 'Percent empty'),
  ('percent_not_empty', 'Percent not empty'),
  ('sum', 'Sum'),
  ('average', 'Average'),
  ('median', 'Median'),
  ('min', 'Min'),
  ('max', 'Max'),
  ('range', 'Range'),
  ('checked', 'Checked'),
  ('unchecked', 'Unchecked'),
  ('percent_checked', 'Percent checked'),
  ('percent_unchecked', 'Percent unchecked'),
  ('earliest_date', 'Earliest date'),
  ('latest_date', 'Latest date'),
  ('date_range', 'Date range'),
];

String _suffix(PropDef d) {
  final f = d.cfg.rollupFunction;
  if (f.startsWith('percent_')) return '%';
  if (f == 'date_range') return ' days';
  return '';
}

final rollupProperty = PropertyType(
  kind: PropKind.rollup,
  label: 'Rollup',
  icon: DbIco.rollup,
  advanced: true,
  format: (v, d) => computedText(v, d, suffix: _suffix(d)),
  cell: (a) => computedCell(a, suffix: _suffix(a.def)),
  filterOps: computedFilterOps,
  filterValue: computedFilterValue,
  calcs: const [Calc.sum, Calc.average, Calc.median, Calc.min, Calc.max, Calc.range],
);

/// The related property's name ("Name" for the title).
Future<String?> _targetName(CollectionStore store, PropDef relation, String? key) async {
  if (key == null) return null;
  if (key == BuiltinKeys.title) return 'Name';
  final schema = await store.vault.run((v) => v.getSchema(collection: relationTarget(relation, store.collection)));
  for (final p in schema) {
    if (p.key == key) return p.name;
  }
  return null;
}

/// The "Relation", "Property" and "Calculate" rows of the property menu's
/// edit page.
Widget rollupSettings(BuildContext context, CollectionStore store, PropDef d, Future<void> Function(PropDef) save) =>
    _RollupSettings(store: store, def: d, save: save);

class _RollupSettings extends StatefulWidget {
  const _RollupSettings({required this.store, required this.def, required this.save});
  final CollectionStore store;
  final PropDef def;
  final Future<void> Function(PropDef) save;

  @override
  State<_RollupSettings> createState() => _RollupSettingsState();
}

class _RollupSettingsState extends State<_RollupSettings> {
  String? _target;

  PropDef get d => widget.def;
  PropDef? get _relation {
    final k = d.cfg.rollupRelation;
    for (final p in widget.store.schema) {
      if (p.key == k && p.kind == PropKind.relation) return p;
    }
    return null;
  }

  @override
  void initState() {
    super.initState();
    _loadTarget();
  }

  @override
  void didUpdateWidget(_RollupSettings old) {
    super.didUpdateWidget(old);
    if (old.def.config != d.config) _loadTarget();
  }

  Future<void> _loadTarget() async {
    final rel = _relation;
    final name = rel == null ? null : await _targetName(widget.store, rel, d.cfg.rollupProperty);
    if (mounted) setState(() => _target = name);
  }

  Future<void> _set(void Function(PropConfig c) f) async {
    final c = d.cfg;
    f(c);
    await widget.save(d.copyWith(config: c.encode()));
  }

  Future<void> _pick<T>(Rect rect, String title, List<(T, String)> items, T? current, ValueChanged<T> pick) =>
      showPanel<void>(
        context,
        rect,
        (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          MenuHeader(title),
          if (items.isEmpty)
            Padding(
              padding: const EdgeInsets.all(8),
              child: Text('No properties to choose', style: ts(context.type.uiSmall, context.colors.textTertiary)),
            ),
          Flexible(
            child: SingleChildScrollView(
              child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                for (final (v, label) in items)
                  MenuItem(
                    key: ValueKey('rollup-choice-$label'),
                    label: label,
                    trailing: v == current ? Icon20(Ico.check, size: 16, color: context.colors.iconPrimary) : null,
                    onTap: () {
                      Navigator.of(ctx).maybePop();
                      pick(v);
                    },
                  ),
              ]),
            ),
          ),
        ]),
        width: 260,
      );

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final rel = _relation;
    final fn = d.cfg.rollupFunction;
    final fnLabel = rollupFunctions.firstWhere((f) => f.$1 == fn, orElse: () => rollupFunctions.first).$2;
    Widget icon(DbIco i) => DbIcon(i, size: context.mobile ? 22 : 20, color: c.iconPrimary);
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      settingsRow(context, 'Relation',
          key: const ValueKey('rollup-relation'),
          value: rel?.name ?? 'Select',
          leading: icon(DbIco.relation),
          onTap: (r) => _pick<String>(
              r,
              'Relation',
              [for (final p in widget.store.schema) if (p.kind == PropKind.relation) (p.key, p.name)],
              rel?.key,
              (k) => _set((c) => c
                ..rollupRelation = k
                ..rollupProperty = null
                ..rollupFunction = c.rollupFunction))),
      settingsRow(context, 'Property',
          key: const ValueKey('rollup-property'),
          value: _target ?? 'Select',
          leading: icon(DbIco.rollup),
          onTap: (r) async {
            if (rel == null) return;
            final schema = await widget.store.vault
                .run((v) => v.getSchema(collection: relationTarget(rel, widget.store.collection)));
            if (!mounted) return;
            await _pick<String>(
                r,
                'Property',
                [(BuiltinKeys.title, 'Name'), for (final p in schema) (p.key, p.name)],
                d.cfg.rollupProperty,
                (k) => _set((c) => c.rollupProperty = k));
          }),
      settingsRow(context, 'Calculate',
          key: const ValueKey('rollup-function'),
          value: fnLabel,
          leading: icon(DbIco.sum),
          onTap: (r) => _pick<String>(r, 'Calculate', rollupFunctions, fn, (f) => _set((c) => c.rollupFunction = f))),
    ]);
  }
}
