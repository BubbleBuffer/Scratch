// Formula: a value the core computes from the row's other properties with
// a small expression language (`prop("Price") * prop("Qty")`, `if(...)`,
// `dateBetween(...)`, ...). Read-only in cells; the expression is edited
// from the property menu. Errors show in the cell.

import 'package:flutter/widgets.dart';

import '../../../core/models.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_icons.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';
import '../../../registry/property_registry.dart';
import '../../../state/collection_store.dart';
import '../../../ui/ui.dart';
import 'computed_values.dart';

final formulaProperty = PropertyType(
  kind: PropKind.formula,
  label: 'Formula',
  icon: DbIco.formula,
  advanced: true,
  format: (v, d) => computedText(v, d),
  cell: (a) => computedCell(a),
  filterOps: computedFilterOps,
  filterValue: computedFilterValue,
  calcs: const [Calc.sum, Calc.average, Calc.median, Calc.min, Calc.max, Calc.range],
);

/// Functions offered under the field: (name, what it does).
const formulaFunctions = [
  ('if', 'if(condition, then, else)'),
  ('concat', 'concat(text, …)'),
  ('length', 'length(text or list)'),
  ('contains', 'contains(text, part)'),
  ('round', 'round(number, places)'),
  ('dateAdd', 'dateAdd(date, amount, "days")'),
  ('dateBetween', 'dateBetween(date, date, "days")'),
  ('formatDate', 'formatDate(date, "MMM D, YYYY")'),
  ('now', 'now()'),
  ('empty', 'empty(value)'),
];

/// The "Formula" row of the property menu's edit page.
Widget formulaSettings(BuildContext context, CollectionStore store, PropDef d, Future<void> Function(PropDef) save) {
  final f = d.cfg.formula;
  return settingsRow(context, 'Formula',
      key: const ValueKey('prop-formula'),
      value: f.isEmpty ? 'Empty' : f,
      leading: DbIcon(DbIco.formula, size: context.mobile ? 22 : 20, color: context.colors.iconPrimary),
      onTap: (rect) => editFormula(context, rect, store, d, save));
}

Future<void> editFormula(BuildContext context, Rect rect, CollectionStore store, PropDef d, Future<void> Function(PropDef) save) =>
    showPanel<void>(context, rect, (ctx) => FormulaEditor(store: store, def: d, save: save), width: 420);

class FormulaEditor extends StatefulWidget {
  const FormulaEditor({super.key, required this.store, required this.def, required this.save});
  final CollectionStore store;
  final PropDef def;
  final Future<void> Function(PropDef) save;

  @override
  State<FormulaEditor> createState() => _FormulaEditorState();
}

class _FormulaEditorState extends State<FormulaEditor> {
  late final _ctl = TextEditingController(text: widget.def.cfg.formula);

  @override
  void dispose() {
    _ctl.dispose();
    super.dispose();
  }

  void _insert(String s) {
    final t = _ctl.text;
    final sel = _ctl.selection;
    final at = sel.isValid ? sel.start : t.length;
    final end = sel.isValid ? sel.end : t.length;
    _ctl.value = TextEditingValue(
      text: t.replaceRange(at, end, s),
      selection: TextSelection.collapsed(offset: at + s.length),
    );
  }

  Future<void> _done() async {
    final c = widget.def.cfg..formula = _ctl.text.trim();
    Navigator.of(context).maybePop();
    await widget.save(widget.def.copyWith(config: c.encode()));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final props = [for (final p in widget.store.schema) if (p.key != widget.def.key) p];
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Row(children: [
        Expanded(
          child: MenuField(
            key: const ValueKey('formula-field'),
            controller: _ctl,
            placeholder: 'Type a formula…',
            autofocus: !context.mobile,
            onSubmitted: (_) => _done(),
          ),
        ),
        const SizedBox(width: 6),
        PrimaryButton(key: const ValueKey('formula-done'), label: 'Done', onTap: _done),
      ]),
      const SizedBox(height: 4),
      Flexible(
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            const MenuHeader('Properties'),
            MenuItem(
              key: const ValueKey('formula-prop-title'),
              label: 'Name',
              leading: DbIcon(DbIco.title, size: 18, color: c.iconSecondary),
              onTap: () => _insert('prop("Name")'),
            ),
            for (final p in props)
              MenuItem(
                key: ValueKey('formula-prop-${p.name}'),
                label: p.name,
                leading: DbIcon(propertyRegistry.of(p.kind)?.icon ?? DbIco.text, size: 18, color: c.iconSecondary),
                onTap: () => _insert('prop("${p.name}")'),
              ),
            const MenuHeader('Functions'),
            for (final (name, usage) in formulaFunctions)
              MenuItem(
                key: ValueKey('formula-fn-$name'),
                label: usage,
                onTap: () => _insert(name == 'now' ? 'now()' : '$name('),
              ),
          ]),
        ),
      ),
    ]);
  }
}
