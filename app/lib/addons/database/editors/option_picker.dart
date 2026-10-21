// Select, multi-select and status values: the option picker (search or
// create, pick, reorder by dragging) and the option editor (name, colour,
// delete).

import 'package:flutter/widgets.dart';

import '../../../core/models.dart' hide StatusGroup;
import '../../../registry/property_registry.dart';
import '../../../ui/ui.dart';
import '../../../ui/database/config.dart';
import '../../../ui/database/db_theme.dart';
import '../../../ui/database/values.dart';
import '../../../ui/database/widgets.dart';

/// The colour a new option gets: the palette in turn, skipping Default.
String nextOptionColor(List<SelectOption> options) => tagColors[1 + options.length % (tagColors.length - 1)];

/// Opens the option picker for [a]: over the cell on desktop (its chips
/// where the cell's were), a sheet on mobile.
Future<void> editOptions(EditArgs a) => showOverCell<void>(
      a.context,
      a.anchor,
      (ctx) => ctx.mobile ? OptionPicker(args: a) : Padding(padding: const EdgeInsets.all(6), child: OptionPicker(args: a)),
      width: a.anchor.width < 300 ? 300 : a.anchor.width,
    );

class OptionPicker extends StatefulWidget {
  const OptionPicker({super.key, required this.args});
  final EditArgs args;

  @override
  State<OptionPicker> createState() => _OptionPickerState();
}

class _OptionPickerState extends State<OptionPicker> {
  late PropDef def = widget.args.def;
  late List<String> selected = widget.args.value.optionIds.toList();
  final _query = TextEditingController();

  bool get multi => def.kind == PropKind.multiSelect;
  bool get status => def.kind == PropKind.status;

  @override
  void dispose() {
    _query.dispose();
    _queryFocus.dispose();
    super.dispose();
  }

  void _emit() {
    final v = multi
        ? Values.multi(selected)
        : status
            ? Values.status(selected.isEmpty ? '' : selected.first)
            : Values.select(selected.isEmpty ? '' : selected.first);
    widget.args.onChanged(v);
  }

  void _pick(String id) {
    setState(() {
      if (multi) {
        selected.contains(id) ? selected.remove(id) : selected.add(id);
      } else {
        selected = selected.contains(id) && !status ? [] : [id];
      }
    });
    _emit();
    if (!multi) Navigator.of(context).maybePop();
  }

  Future<void> _create(String name) async {
    final before = def.options.map((o) => o.id).toSet();
    final next = def.copyWith(options: [
      ...def.options,
      SelectOption(id: '', name: name, color: nextOptionColor(def.options)),
    ]);
    final saved = await widget.args.saveDef(next);
    if (!mounted) return;
    final made = saved.options.firstWhere((o) => !before.contains(o.id), orElse: () => saved.options.last);
    setState(() {
      def = saved;
      _query.clear();
    });
    _pick(made.id);
  }

  Future<void> _save(PropDef next) async {
    setState(() => def = next);
    final saved = await widget.args.saveDef(next);
    if (!mounted) return;
    setState(() {
      def = saved;
      selected.removeWhere((id) => saved.option(id) == null);
    });
  }

  Future<void> _editOption(BuildContext row, SelectOption o) async {
    final r = await showPanel<_OptionEdit>(context, globalRectOf(row), (ctx) => OptionEditor(option: o), width: 220);
    if (r == null || !mounted) return;
    if (r.delete) {
      await _save(def.copyWith(options: [for (final x in def.options) if (x.id != o.id) x]));
      _emit();
    } else {
      await _save(def.copyWith(options: [
        for (final x in def.options) x.id == o.id ? SelectOption(id: o.id, name: r.name, color: r.color) : x,
      ]));
    }
  }

  /// Moves shown option [from] to [to] (an index after the removal); the
  /// shown options keep the slots they had among all options.
  void _reorder(List<SelectOption> shown, int from, int to) {
    final next = [...shown];
    final moving = next.removeAt(from);
    next.insert(to.clamp(0, next.length), moving);
    final ids = shown.map((o) => o.id).toSet();
    var k = 0;
    final list = [for (final o in def.options) ids.contains(o.id) ? next[k++] : o];
    _save(def.copyWith(options: list));
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final q = _query.text.trim().toLowerCase();
    final shown = [for (final o in def.options) if (q.isEmpty || o.name.toLowerCase().contains(q)) o];
    final exact = def.options.any((o) => o.name.toLowerCase() == q);

    Widget row(SelectOption o, int i) => Builder(
          key: ValueKey('opt-${o.id}'),
          builder: (rowCtx) => Hover(
            onTap: () => _pick(o.id),
            radius: Radii.md,
            selected: selected.contains(o.id),
            padding: const EdgeInsets.symmetric(horizontal: 4),
            child: SizedBox(
              height: context.mobile ? 44 : 28,
              child: Row(children: [
                ReorderableDragStartListener(
                  index: i,
                  child: MouseRegion(
                    cursor: SystemMouseCursors.grab,
                    child: SizedBox(width: 18, child: Icon20(Ico.grip, size: 14, color: c.iconTertiary)),
                  ),
                ),
                const SizedBox(width: 4),
                Expanded(child: Align(alignment: Alignment.centerLeft, child: Pill(o.name, color: o.color, status: status,
                    group: status ? statusGroupOf(def, o.id) : null))),
                IconBtn(Ico.more,
                    key: ValueKey('opt-more-${o.name}'),
                    size: 22,
                    glyph: 16,
                    onTap: () => _editOption(rowCtx, o),
                    semanticLabel: 'Edit option'),
              ]),
            ),
          ),
        );

    Widget list;
    if (status && q.isEmpty) {
      final byGroup = <StatusGroup, List<SelectOption>>{};
      for (final o in def.options) {
        byGroup.putIfAbsent(statusGroupOf(def, o.id), () => []).add(o);
      }
      list = Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
        for (final g in StatusGroup.values) ...[
          MenuHeader(g.label),
          for (final o in byGroup[g] ?? const <SelectOption>[]) row(o, def.options.indexOf(o)),
        ],
      ]);
    } else {
      list = ReorderableList(
        shrinkWrap: true,
        physics: const NeverScrollableScrollPhysics(),
        itemCount: shown.length,
        onReorderItem: (a, b) => _reorder(shown, a, b),
        itemBuilder: (context, i) => row(shown[i], i),
      );
    }

    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Container(
        constraints: const BoxConstraints(minHeight: 34),
        padding: const EdgeInsets.fromLTRB(6, 4, 6, 4),
        decoration: BoxDecoration(
          color: c.bgSecondary,
          borderRadius: Radii.field,
          border: Border.all(color: c.borderTranslucent),
        ),
        child: Wrap(spacing: 6, runSpacing: 4, crossAxisAlignment: WrapCrossAlignment.center, children: [
          for (final id in selected)
            if (def.option(id) case final o?)
              Row(mainAxisSize: MainAxisSize.min, children: [
                Pill(o.name, color: o.color, status: status,
                        group: status ? statusGroupOf(def, o.id) : null),
                if (multi)
                  Hover(
                    onTap: () => _pick(id),
                    radius: 3,
                    child: Icon20(Ico.close, size: 14, color: c.iconTertiary),
                  ),
              ]),
          ConstrainedBox(
            constraints: const BoxConstraints(minWidth: 80, maxWidth: 260),
            child: IntrinsicWidth(
              child: PlainField(
                key: const ValueKey('option-query'),
                controller: _query,
                focusNode: _queryFocus,
                autofocus: !context.mobile,
                style: ts(ty.ui, c.textPrimary, weight: 400),
                placeholder: selected.isEmpty ? 'Search for an option…' : '',
                onChanged: (_) => setState(() {}),
                onSubmitted: (s) {
                  if (s.trim().isEmpty) return;
                  if (shown.isNotEmpty && (exact || shown.length == 1)) {
                    _pick(shown.first.id);
                  } else {
                    _create(s.trim());
                  }
                },
              ),
            ),
          ),
        ]),
      ),
      const SizedBox(height: 6),
      Padding(
        padding: const EdgeInsets.fromLTRB(4, 2, 4, 4),
        child: Text('Select an option or create one',
            style: ts(ty.uiSmall, c.textTertiary, weight: 400)),
      ),
      Flexible(child: SingleChildScrollView(child: list)),
      if (q.isNotEmpty && !exact)
        Hover(
          key: const ValueKey('option-create'),
          onTap: () => _create(_query.text.trim()),
          radius: Radii.md,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          child: SizedBox(
            height: context.mobile ? 44 : 28,
            child: Row(children: [
              Text('Create ', style: ts(ty.ui, c.textSecondary, weight: 400)),
              Flexible(child: Pill(_query.text.trim(), color: nextOptionColor(def.options), status: status)),
            ]),
          ),
        ),
    ]);
  }

  final _queryFocus = FocusNode();
}

class _OptionEdit {
  const _OptionEdit(this.name, this.color, {this.delete = false});
  final String name, color;
  final bool delete;
}

/// Name, Delete and the colour list of one option.
class OptionEditor extends StatefulWidget {
  const OptionEditor({super.key, required this.option});
  final SelectOption option;

  @override
  State<OptionEditor> createState() => _OptionEditorState();
}

class _OptionEditorState extends State<OptionEditor> {
  late final _name = TextEditingController(text: widget.option.name);
  late String color = widget.option.color.isEmpty ? 'default' : widget.option.color;

  @override
  void dispose() {
    _name.dispose();
    super.dispose();
  }

  void _done() => Navigator.of(context).pop(_OptionEdit(_name.text.trim().isEmpty ? widget.option.name : _name.text.trim(), color));

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        MenuField(key: const ValueKey('option-name'), controller: _name, onSubmitted: (_) => _done()),
        const SizedBox(height: 4),
        dbItem(context, 'Delete', null,
            key: const ValueKey('option-delete'),
            ico: Ico.trash,
            onTap: () => Navigator.of(context).pop(_OptionEdit(widget.option.name, color, delete: true))),
        const MenuSeparator(),
        const MenuHeader('Colors'),
        for (final col in tagColors)
          MenuItem(
            key: ValueKey('color-$col'),
            label: colorLabel(col),
            leading: Container(
              width: 18,
              height: 18,
              decoration: BoxDecoration(
                color: c.tag(col).bg,
                borderRadius: BorderRadius.circular(4),
                border: Border.all(color: c.borderTranslucent),
              ),
            ),
            trailing: col == color ? Icon20(Ico.check, size: 16, color: c.iconPrimary) : null,
            onTap: () {
              setState(() => color = col);
              _done();
            },
          ),
      ]);
  }
}

/// The option editor for a property's option (from the property menu):
/// returns the edited option, `null` when unchanged, or deletes it.
Future<void> editOptionOf(BuildContext context, Rect anchor, PropDef def, SelectOption o,
    Future<PropDef> Function(PropDef) save) async {
  final r = await showPanel<_OptionEdit>(context, anchor, (ctx) => OptionEditor(option: o), width: 220);
  if (r == null) return;
  if (r.delete) {
    await save(def.copyWith(options: [for (final x in def.options) if (x.id != o.id) x]));
  } else if (r.name != o.name || r.color != o.color) {
    await save(def.copyWith(options: [
      for (final x in def.options) x.id == o.id ? SelectOption(id: o.id, name: r.name, color: r.color) : x,
    ]));
  }
}
