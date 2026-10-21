// Filters and sorts of a view: the property picker ("Filter by…"), the
// rule editor of a simple filter ("Status  is ▾  [value]"), the advanced
// filter (Where / And / Or rows, one level of groups), and the sort editor
// ("Property ▾  Ascending ▾  ×", "+ Add sort", "Delete sort").

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../ui/database/config.dart';
import '../../registry/property_registry.dart';
import '../../state/collection_store.dart';
import '../../ui/ui.dart';
import '../../ui/database/db_icons.dart';
import 'property_menu.dart';
import '../../ui/database/values.dart';
import '../../ui/database/widgets.dart';

/// The title as a property definition (for filters and sorts).
const titleDef = PropDef(key: BuiltinKeys.title, name: 'Name', kind: PropKind.text, options: [], config: '');

PropDef? defOf(CollectionStore s, String key) => key == BuiltinKeys.title ? titleDef : s.def(key);

String propName(CollectionStore s, String key) => defOf(s, key)?.name ?? key;

DbIco propIconOf(CollectionStore s, String key) => key == BuiltinKeys.title ? DbIco.title : propIcon(s.def(key));

/// The value a new rule starts with.
PropValue _initialValue(PropDef d, FilterOp op) => Values.empty;

Condition newRule(PropDef d) {
  final t = propertyRegistry.of(d.kind);
  final op = (t?.filterOps.isNotEmpty ?? false) ? t!.filterOps.first : FilterOp.isNotEmpty;
  return Condition(prop: d.key, op: op, value: _initialValue(d, op));
}

/// Picks a property ("Filter by…" / "Sort by…"); completes with its key.
Future<String?> pickProperty(BuildContext context, Rect rect, CollectionStore s, {required String placeholder, Set<String> exclude = const {}}) =>
    showPanel<String>(context, rect, (ctx) => _PropPicker(store: s, placeholder: placeholder, exclude: exclude), width: 290);

class _PropPicker extends StatefulWidget {
  const _PropPicker({required this.store, required this.placeholder, required this.exclude});
  final CollectionStore store;
  final String placeholder;
  final Set<String> exclude;

  @override
  State<_PropPicker> createState() => _PropPickerState();
}

class _PropPickerState extends State<_PropPicker> {
  String q = '';

  @override
  Widget build(BuildContext context) {
    final defs = [titleDef, ...widget.store.schema];
    final shown = [
      for (final d in defs)
        if (!widget.exclude.contains(d.key) && (q.isEmpty || d.name.toLowerCase().contains(q))) d,
    ];
    return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      MenuField(
        key: const ValueKey('prop-picker-search'),
        placeholder: widget.placeholder,
        autofocus: !context.mobile,
        onChanged: (s) => setState(() => q = s.trim().toLowerCase()),
        onSubmitted: (_) {
          if (shown.isNotEmpty) Navigator.of(context).pop(shown.first.key);
        },
      ),
      const SizedBox(height: 4),
      Flexible(
        child: SingleChildScrollView(
          child: Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            for (var i = 0; i < shown.length; i++)
              dbItem(context, shown[i].name, shown[i].key == BuiltinKeys.title ? DbIco.title : propIcon(shown[i]),
                  key: ValueKey('pick-${shown[i].name}'),
                  selected: i == 0 && !context.mobile,
                  onTap: () => Navigator.of(context).pop(shown[i].key)),
          ]),
        ),
      ),
    ]);
  }
}

/// [v]'s rules when its filter is edited as simple rules; null for an
/// advanced filter.
List<Condition>? simpleRulesOf(ViewDef? v) =>
    v == null ? const [] : (v.cfg.advancedFilter ? null : Filters.simpleRules(v.filter));

ViewDef _withAdvanced(ViewDef v, bool on) {
  final c = v.cfg..advancedFilter = on;
  return v.copyWith(config: c.encode());
}

final _ruleChips = Expando<Map<int, GlobalKey>>();

/// The key of chip [i] of [s]'s rules bar (-1: the advanced filter chip),
/// so a rule just added opens its editor at its chip.
GlobalKey ruleChipKey(CollectionStore s, int i) => (_ruleChips[s] ??= {})[i] ??= GlobalKey();

/// Chip [i] of [s]'s rules bar once the bar has rebuilt (null if not
/// shown).
Future<BuildContext?> _chip(CollectionStore s, int i) async {
  await SchedulerBinding.instance.endOfFrame;
  final c = ruleChipKey(s, i).currentContext;
  return c != null && c.mounted ? c : null;
}

/// Adds a simple rule on [key] to the active view and opens its editor at
/// the rule's chip.
Future<void> addFilterFor(BuildContext anchor, CollectionStore s, String key) async {
  final v = s.view;
  final d = defOf(s, key);
  if (v == null || d == null) return;
  final rule = newRule(d);
  final rules = simpleRulesOf(v);
  if (rules == null) {
    // An advanced filter: the rule joins it.
    final f = v.filter!;
    final next = f.kind == FilterKind.and
        ? Filters.and([...f.children, Filters.cond(rule.prop, rule.op, rule.value)])
        : Filters.and([f, Filters.cond(rule.prop, rule.op, rule.value)]);
    await s.updateView(v.copyWith(filter: next));
    final chip = await _chip(s, -1);
    if (chip != null && chip.mounted) {
      await showAdvancedFilter(chip, globalRectOf(chip), s);
    } else if (anchor.mounted) {
      await showAdvancedFilter(anchor, globalRectOf(anchor), s);
    }
    return;
  }
  final next = [...rules, rule];
  await s.updateView(v.copyWith(filter: Filters.ofRules(next)));
  final chip = await _chip(s, next.length - 1);
  if (chip != null && chip.mounted) {
    await showRuleEditor(chip, globalRectOf(chip), s, next.length - 1);
  } else if (anchor.mounted) {
    await showRuleEditor(anchor, globalRectOf(anchor), s, next.length - 1);
  }
}

/// The toolbar's Filter button: pick a property, add a rule.
Future<void> filterButton(BuildContext anchor, CollectionStore s) async {
  final key = await pickProperty(anchor, globalRectOf(anchor), s, placeholder: 'Filter by…');
  if (key == null || !anchor.mounted) return;
  await addFilterFor(anchor, s, key);
}

/// The editor of simple rule [index] of the active view.
Future<void> showRuleEditor(BuildContext context, Rect rect, CollectionStore s, int index) =>
    showPanel<void>(context, rect, (ctx) => _RuleEditor(store: s, index: index), width: 300);

class _RuleEditor extends StatelessWidget {
  const _RuleEditor({required this.store, required this.index});
  final CollectionStore store;
  final int index;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final v = store.view;
        final rules = simpleRulesOf(v) ?? const [];
        if (v == null || index >= rules.length) return const SizedBox.shrink();
        final r = rules[index];
        void save(Condition c) {
          final next = [...rules];
          next[index] = c;
          store.updateView(v.copyWith(filter: Filters.ofRules(next)));
        }

        return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Row(children: [
            Padding(
              padding: const EdgeInsets.only(left: 4),
              child: Text(propName(store, r.prop), style: ts(context.type.uiSmall, context.colors.textSecondary)),
            ),
            const SizedBox(width: 4),
            _OpButton(store: store, rule: r, onChanged: save),
            const Spacer(),
            Builder(
              builder: (b) => IconBtn(Ico.more,
                  key: const ValueKey('rule-more'),
                  size: 24,
                  glyph: 16,
                  semanticLabel: 'Rule options',
                  onTap: () async {
                    final pick = await showPanel<String>(
                      b,
                      globalRectOf(b),
                      (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                        MenuItem(
                            key: const ValueKey('rule-delete'),
                            label: 'Delete filter',
                            icon: Ico.trash,
                            onTap: () => Navigator.of(ctx).pop('delete')),
                        dbItem(ctx, 'Add to advanced filter', DbIco.filter,
                            key: const ValueKey('rule-advanced'), onTap: () => Navigator.of(ctx).pop('advanced')),
                      ]),
                      width: 240,
                    );
                    if (pick == 'delete') {
                      final next = [...rules]..removeAt(index);
                      await store.updateView(v.copyWith(filter: Filters.ofRules(next), clearFilter: next.isEmpty));
                      if (context.mounted) Navigator.of(context).pop();
                    } else if (pick == 'advanced') {
                      await store.updateView(_withAdvanced(
                          v.copyWith(filter: Filters.and([for (final x in rules) Filters.cond(x.prop, x.op, x.value)])), true));
                      if (context.mounted) {
                        final rect = globalRectOf(context);
                        final nav = Navigator.of(context);
                        nav.pop();
                        // At the "N rules" chip that replaced the rule's.
                        final chip = await _chip(store, -1);
                        if (chip != null && chip.mounted) {
                          await showAdvancedFilter(chip, globalRectOf(chip), store);
                        } else if (nav.mounted) {
                          await showAdvancedFilter(nav.context, rect, store);
                        }
                      }
                    }
                  }),
            ),
          ]),
          const SizedBox(height: 4),
          if (opNeedsValue(r.op)) _valueInput(context, store, r, save),
        ]);
      },
    );
  }
}

Widget _valueInput(BuildContext context, CollectionStore s, Condition r, ValueChanged<Condition> save) {
  final d = defOf(s, r.prop);
  final t = d == null ? null : propertyRegistry.of(d.kind);
  final input = t?.filterValue;
  if (d == null || input == null) return const SizedBox.shrink();
  return input(FilterValueArgs(
    context: context,
    def: d,
    op: r.op,
    value: r.value,
    onChanged: (v) => save(Condition(prop: r.prop, op: r.op, value: v)),
  ));
}

/// The op dropdown of a rule ("is ▾").
class _OpButton extends StatelessWidget {
  const _OpButton({required this.store, required this.rule, required this.onChanged});
  final CollectionStore store;
  final Condition rule;
  final ValueChanged<Condition> onChanged;

  @override
  Widget build(BuildContext context) {
    final d = defOf(store, rule.prop);
    final ops = d == null ? const <FilterOp>[] : (propertyRegistry.of(d.kind)?.filterOps ?? const <FilterOp>[]);
    return Builder(
      builder: (b) => ChipBtn(
        key: const ValueKey('rule-op'),
        label: filterOpLabel(rule.op, d?.kind).toLowerCase(),
        chevron: true,
        radius: Radii.md,
        onTap: () async {
          final op = await showPanel<FilterOp>(
            b,
            globalRectOf(b),
            (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              for (final o in ops)
                MenuItem(
                  key: ValueKey('op-${o.name}'),
                  label: filterOpLabel(o, d?.kind),
                  selected: o == rule.op,
                  onTap: () => Navigator.of(ctx).pop(o),
                ),
            ]),
            width: 220,
          );
          if (op == null) return;
          final keep = opNeedsValue(op) ? rule.value : Values.empty;
          onChanged(Condition(prop: rule.prop, op: op, value: keep));
        },
      ),
    );
  }
}

// ----------------------------------------------------------- advanced filter

Future<void> showAdvancedFilter(BuildContext context, Rect rect, CollectionStore s) =>
    showPanel<void>(context, rect, (ctx) => _AdvancedFilter(store: s), width: context.mobile ? 360 : 560);

class _AdvancedFilter extends StatelessWidget {
  const _AdvancedFilter({required this.store});
  final CollectionStore store;

  /// Path of child indexes from the root to a node.
  static Filter _replace(Filter root, List<int> path, Filter? node) {
    if (path.isEmpty) return node ?? Filters.and(const []);
    final kids = [...root.children];
    if (path.length == 1) {
      if (node == null) {
        kids.removeAt(path.first);
      } else {
        kids[path.first] = node;
      }
    } else {
      kids[path.first] = _replace(kids[path.first], path.sublist(1), node);
    }
    return Filter(kind: root.kind, children: kids, cond: root.cond);
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final v = store.view;
        if (v == null) return const SizedBox.shrink();
        var root = v.filter ?? Filters.and(const []);
        if (root.kind == FilterKind.cond) root = Filters.and([root]);
        void save(Filter next) {
          final empty = next.children.isEmpty && next.kind != FilterKind.cond;
          store.updateView(empty ? v.copyWith(clearFilter: true) : v.copyWith(filter: next));
        }

        return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          ..._rows(context, root, const [], save, root),
          const SizedBox(height: 4),
          Row(children: [
            Builder(
              builder: (b) => TextBtn('+ Add filter rule',
                  key: const ValueKey('adv-add-rule'),
                  color: context.colors.textSecondary,
                  onTap: () => save(Filter(kind: root.kind, children: [
                        ...root.children,
                        Filters.cond(BuiltinKeys.title, FilterOp.contains, Values.empty),
                      ]))),
            ),
            TextBtn('+ Add filter group',
                key: const ValueKey('adv-add-group'),
                color: context.colors.textSecondary,
                onTap: () => save(Filter(kind: root.kind, children: [
                      ...root.children,
                      Filters.or([Filters.cond(BuiltinKeys.title, FilterOp.contains, Values.empty)]),
                    ]))),
          ]),
          TextBtn('Delete filter',
              key: const ValueKey('adv-delete'),
              color: context.colors.inlineCodeText,
              onTap: () {
                store.updateView(_withAdvanced(v.copyWith(clearFilter: true), false));
                Navigator.of(context).maybePop();
              }),
        ]);
      },
    );
  }

  List<Widget> _rows(BuildContext context, Filter group, List<int> path, ValueChanged<Filter> save, Filter root) {
    final out = <Widget>[];
    for (var i = 0; i < group.children.length; i++) {
      final child = group.children[i];
      final p = [...path, i];
      final lead = i == 0
          ? Text('Where', style: ts(context.type.ui, context.colors.textSecondary, weight: 400))
          : i == 1
              ? Builder(
                  builder: (b) => ChipBtn(
                    key: ValueKey('adv-join-${path.length}'),
                    label: group.kind == FilterKind.or ? 'Or' : 'And',
                    chevron: true,
                    radius: Radii.md,
                    onTap: () => save(_replace(root, path,
                        Filter(kind: group.kind == FilterKind.or ? FilterKind.and : FilterKind.or, children: group.children))),
                  ),
                )
              : Text(group.kind == FilterKind.or ? 'Or' : 'And',
                  style: ts(context.type.ui, context.colors.textSecondary, weight: 400));
      Widget body;
      if (child.kind == FilterKind.cond && child.cond != null) {
        final r = child.cond!;
        body = Row(children: [
          Builder(
            builder: (b) => ChipBtn(
              key: ValueKey('adv-prop-${p.join('-')}'),
              label: propName(store, r.prop),
              icon: propIconOf(store, r.prop),
              chevron: true,
              radius: Radii.md,
              onTap: () async {
                final key = await pickProperty(b, globalRectOf(b), store, placeholder: 'Filter by…');
                final d = key == null ? null : defOf(store, key);
                if (d == null) return;
                final nr = newRule(d);
                save(_replace(root, p, Filters.cond(nr.prop, nr.op, nr.value)));
              },
            ),
          ),
          const SizedBox(width: 4),
          _OpButton(store: store, rule: r, onChanged: (c) => save(_replace(root, p, Filters.cond(c.prop, c.op, c.value)))),
          const SizedBox(width: 4),
          if (opNeedsValue(r.op))
            Expanded(
              child: Builder(
                builder: (b) => _AdvValue(
                  store: store,
                  rule: r,
                  onChanged: (c) => save(_replace(root, p, Filters.cond(c.prop, c.op, c.value))),
                ),
              ),
            )
          else
            const Spacer(),
          IconBtn(Ico.close,
              key: ValueKey('adv-remove-${p.join('-')}'),
              size: 24,
              glyph: 14,
              semanticLabel: 'Remove rule',
              onTap: () => save(_replace(root, p, null))),
        ]);
      } else {
        body = Container(
          padding: const EdgeInsets.all(6),
          decoration: BoxDecoration(
            color: context.colors.bgHover,
            borderRadius: BorderRadius.circular(Radii.md),
          ),
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
            ..._rows(context, child, p, save, root),
            TextBtn('+ Add filter rule',
                color: context.colors.textSecondary,
                onTap: () => save(_replace(root, p, Filter(kind: child.kind, children: [
                      ...child.children,
                      Filters.cond(BuiltinKeys.title, FilterOp.contains, Values.empty),
                    ])))),
          ]),
        );
      }
      out.add(Padding(
        padding: const EdgeInsets.symmetric(vertical: 2),
        child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
          SizedBox(width: 64, height: 28, child: Align(alignment: Alignment.centerLeft, child: lead)),
          Expanded(child: body),
        ]),
      ));
    }
    return out;
  }
}

/// The value of an advanced rule: a field for text and numbers, a button
/// opening the option list or date picker for the rest.
class _AdvValue extends StatelessWidget {
  const _AdvValue({required this.store, required this.rule, required this.onChanged});
  final CollectionStore store;
  final Condition rule;
  final ValueChanged<Condition> onChanged;

  @override
  Widget build(BuildContext context) {
    final d = defOf(store, rule.prop);
    if (d == null) return const SizedBox.shrink();
    final textual = !(d.hasOptions || d.kind == PropKind.date || d.computed);
    if (textual) return _valueInput(context, store, rule, onChanged);
    final t = propertyRegistry.of(d.kind);
    final label = rule.value.isEmpty ? 'Value' : (t?.format(rule.value, d) ?? '');
    return Builder(
      builder: (b) => ChipBtn(
        label: label,
        chevron: true,
        radius: Radii.md,
        onTap: () => showPanel<void>(b, globalRectOf(b), (ctx) => _valueInput(ctx, store, rule, (c) {
              onChanged(c);
              Navigator.of(ctx).maybePop();
            }), width: 280),
      ),
    );
  }
}

// -------------------------------------------------------------------- sorts

/// The toolbar's Sort button: the sort editor, or (no sorts yet) a
/// property picker that adds the first sort.
Future<void> sortButton(BuildContext anchor, CollectionStore s) async {
  final v = s.view;
  if (v == null) return;
  if (v.sorts.isEmpty) {
    final key = await pickProperty(anchor, globalRectOf(anchor), s, placeholder: 'Sort by…');
    if (key == null) return;
    await s.updateView(v.copyWith(sorts: [SortSpec(prop: key, descending: false)]));
  }
  if (anchor.mounted) await showSortEditor(anchor, globalRectOf(anchor), s);
}

Future<void> showSortEditor(BuildContext context, Rect rect, CollectionStore s) =>
    showPanel<void>(context, rect, (ctx) => _SortEditor(store: s), width: context.mobile ? 360 : 420);

class _SortEditor extends StatelessWidget {
  const _SortEditor({required this.store});
  final CollectionStore store;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) {
        final v = store.view;
        if (v == null) return const SizedBox.shrink();
        final sorts = v.sorts;
        void save(List<SortSpec> next) =>
            store.updateView(v.copyWith(sorts: next));
        return Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          for (var i = 0; i < sorts.length; i++)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 2),
              child: Row(children: [
                Builder(
                  builder: (b) => ChipBtn(
                    key: ValueKey('sort-prop-$i'),
                    label: propName(store, sorts[i].prop),
                    icon: propIconOf(store, sorts[i].prop),
                    chevron: true,
                    radius: Radii.md,
                    onTap: () async {
                      final key = await pickProperty(b, globalRectOf(b), store,
                          placeholder: 'Sort by…', exclude: {for (final s in sorts) if (s.prop != sorts[i].prop) s.prop});
                      if (key == null) return;
                      final next = [...sorts];
                      next[i] = SortSpec(prop: key, descending: sorts[i].descending);
                      save(next);
                    },
                  ),
                ),
                const SizedBox(width: 6),
                Builder(
                  builder: (b) => ChipBtn(
                    key: ValueKey('sort-dir-$i'),
                    label: sorts[i].descending ? 'Descending' : 'Ascending',
                    chevron: true,
                    radius: Radii.md,
                    onTap: () async {
                      final desc = await showPanel<bool>(
                        b,
                        globalRectOf(b),
                        (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                          MenuItem(key: const ValueKey('dir-asc'), label: 'Ascending', onTap: () => Navigator.of(ctx).pop(false)),
                          MenuItem(key: const ValueKey('dir-desc'), label: 'Descending', onTap: () => Navigator.of(ctx).pop(true)),
                        ]),
                        width: 180,
                      );
                      if (desc == null) return;
                      final next = [...sorts];
                      next[i] = SortSpec(prop: sorts[i].prop, descending: desc);
                      save(next);
                    },
                  ),
                ),
                const Spacer(),
                IconBtn(Ico.close,
                    key: ValueKey('sort-remove-$i'),
                    size: 24,
                    glyph: 14,
                    semanticLabel: 'Remove sort',
                    onTap: () => save([...sorts]..removeAt(i))),
              ]),
            ),
          const SizedBox(height: 4),
          Builder(
            builder: (b) => dbItem(context, 'Add sort', null, ico: Ico.plus, key: const ValueKey('sort-add'), onTap: () async {
              final key = await pickProperty(b, globalRectOf(b), store,
                  placeholder: 'Sort by…', exclude: {for (final s in sorts) s.prop});
              if (key == null) return;
              save([...sorts, SortSpec(prop: key, descending: false)]);
            }),
          ),
          dbItem(context, 'Delete sort', null, ico: Ico.trash, key: const ValueKey('sort-delete'), onTap: () {
            save(const []);
            Navigator.of(context).maybePop();
          }),
        ]);
      },
    );
  }
}
