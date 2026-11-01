// The frame around a view: the title, the view tabs (or the mobile view
// switcher), the toolbar (filter, sort, search, view settings, "New" with
// templates), the bar of filter and sort pills, and the view's body from the
// view registry. Used for view blocks (tabs without a title, the views read
// from the blocks through a [ViewSource]) and old inline database blocks.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../services/legacy_host.dart';
import '../../registry/view_registry.dart';
import '../../state/app_state.dart';
import '../../state/collection_store.dart';
import '../../state/pages_store.dart';
import '../../ui/ui.dart';
import 'controller.dart';
import '../../ui/database/db_icons.dart';
import 'filter_editor.dart';
import '../../ui/database/values.dart';
import 'view_settings.dart';
import '../../ui/database/widgets.dart';

/// The active tab of each group of view blocks in this session: the
/// group's first block id -> the shown view's block id.
final viewBlockTabs = <String, String>{};

class DatabaseView extends StatefulWidget {
  const DatabaseView({
    super.key,
    required this.collection,
    this.inline = false,
    this.title,
    this.onTitleChanged,
    this.left = 0,
    this.width,
    this.viewSource,
    this.tabsKey,
    this.onActiveView,
  });

  /// The page whose child pages are shown.
  final String collection;

  /// View blocks: the views come from the blocks; no title is shown.
  final ViewSource? viewSource;

  /// View blocks: the key of the active tab in [viewBlockTabs].
  final String? tabsKey;

  /// View blocks: called with the shown view's id when it changes.
  final ValueChanged<String>? onActiveView;

  bool get blocks => viewSource != null;

  /// An inline database block (sized by its content, inside a page).
  final bool inline;

  /// Full page: the page title's controller (owned by the page screen).
  final TextEditingController? title;
  final ValueChanged<String>? onTitleChanged;

  /// Inline: the page's text column.
  final double left;
  final double? width;

  @override
  State<DatabaseView> createState() => DatabaseViewState();
}

class DatabaseViewState extends State<DatabaseView> {
  CollectionStore? _store;
  late DbController ctl;
  final _search = TextEditingController();
  final _searchFocus = FocusNode();
  bool _searching = false;
  TextEditingController? _inlineTitle;
  MobilePageEntryScope? _historyEntry;

  CollectionStore get store => _store!;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_store == null) {
      final app = context.app;
      _store = CollectionStore(app.vault, app.feed, widget.collection, viewSource: widget.viewSource);
      final tabs = widget.tabsKey;
      if (widget.blocks && tabs != null) {
        store.activeView = viewBlockTabs[tabs];
        store.addListener(_rememberTab);
      }
      _historyEntry = widget.blocks ? null : MobilePageEntryScope.maybeOf(context);
      final entry = _historyEntry;
      if (entry != null) {
        store.activeView = entry.entry.databaseViews[widget.collection];
        if (!entry.preview) store.addListener(_rememberView);
      }
      ctl = DbController(_store!);
      store.load();
    }
  }

  String? _lastTab;

  void _rememberTab() {
    final id = store.view?.id;
    if (id == null || id == _lastTab) return;
    _lastTab = id;
    viewBlockTabs[widget.tabsKey!] = id;
    widget.onActiveView?.call(id);
  }

  void _rememberView() {
    final id = store.view?.id;
    if (id != null) _historyEntry!.entry.databaseViews[widget.collection] = id;
  }

  @override
  void dispose() {
    store.removeListener(_rememberView);
    store.removeListener(_rememberTab);
    store.dispose();
    _search.dispose();
    _searchFocus.dispose();
    _inlineTitle?.dispose();
    _titleFocus.dispose();
    super.dispose();
  }

  double _gutter(double width) {
    if (widget.inline) return widget.left;
    if (context.mobile) return Layout.gutter;
    return width >= 900 ? 96 : 48;
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: store,
      builder: (context, _) => widget.blocks
          ? _blocksFrame(context)
          : widget.inline
              ? _inlineFrame(context)
              : _pageFrame(context),
    );
  }

  // ------------------------------------------------------------ full page

  Widget _pageFrame(BuildContext context) {
    final c = context.colors;
    return ColoredBox(
          color: store.view?.kind == ViewKind.board ? c.desk : c.bgPrimary,
          child: LayoutBuilder(builder: (context, box) {
              final g = _gutter(box.maxWidth);
              final v = store.view;
              return Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
                Padding(padding: EdgeInsets.fromLTRB(g, context.mobile ? 16 : 12, g, 0), child: _pageTitle(context)),
                SizedBox(height: context.mobile ? 12 : 10),
                Padding(
                  padding: EdgeInsets.symmetric(horizontal: g - (context.mobile ? 0 : 0)),
                  child: context.mobile ? _mobileBar(context) : _tabsBar(context, box.maxWidth - 2 * g),
                ),
                if (v != null) _rulesBar(context, v, g),
                const SizedBox(height: 4),
                Expanded(child: _body(context, g)),
              ]);
            }),
        );
  }

  /// The database page's name: its title, else the first line of its body.
  String _displayName(BuildContext context) {
    if (store.title.trim().isNotEmpty) return displayTitle(store.title);
    final pages = context.watch<PagesStore>();
    return displayTitle(store.title, firstLine: pages.meta(store.collection)?.firstLine ?? '');
  }

  Widget _pageTitle(BuildContext context) {
    final c = context.colors;
    final style = tsDisplay(context.mobile ? const TypeSpec(28, 34, 700) : const TypeSpec(32, 40, 700), c.textPrimary);
    final ctl = widget.title;
    if (ctl == null) return Text(_displayName(context), style: style);
    return ListenableBuilder(
      listenable: ctl,
      builder: (context, _) => Stack(children: [
        if (ctl.text.isEmpty)
          Positioned.fill(child: IgnorePointer(child: Text('Untitled', maxLines: 1, style: style.copyWith(color: c.textTertiary)))),
        EditableText(
          key: const ValueKey('db-title'),
          controller: ctl,
          focusNode: _titleFocus,
          style: style,
          cursorColor: c.textPrimary,
          backgroundCursorColor: c.textTertiary,
          selectionColor: c.textSelection,
          cursorWidth: 1.5,
          maxLines: null,
          selectionControls: noteSelectionControls,
          onChanged: widget.onTitleChanged,
        ),
      ]),
    );
  }

  final _titleFocus = FocusNode();

  // ---------------------------------------------------------- view blocks

  /// The tabs, the rules bar and the view; the page's own title and body
  /// are above.
  Widget _blocksFrame(BuildContext context) {
    final w = widget.width ?? 600;
    final v = store.view;
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
      Padding(
        padding: EdgeInsets.only(left: widget.left, right: 8),
        child: SizedBox(
          width: w,
          child: context.mobile ? _mobileBar(context) : _tabsBar(context, w),
        ),
      ),
      if (v != null) _rulesBar(context, v, widget.left),
      _body(context, widget.left),
    ]);
  }

  /// A new List view after the view blocks, shown at once.
  Future<void> addListView() => store.addView(ViewKind.list);

  // --------------------------------------------------------------- inline

  Widget _inlineFrame(BuildContext context) {
    final c = context.colors;
    final w = widget.width ?? 600;
    final v = store.view;
    _inlineTitle ??= TextEditingController(text: store.title);
    if (!_titleFocus.hasFocus && _inlineTitle!.text != store.title && store.loaded) _inlineTitle!.text = store.title;
    final style = tsRaw(context.mobile ? 18 : 20, 28, 700, c.textPrimary);
    return Column(crossAxisAlignment: CrossAxisAlignment.stretch, mainAxisSize: MainAxisSize.min, children: [
      Padding(
        padding: EdgeInsets.only(left: widget.left, right: 8),
        child: SizedBox(
          width: w,
          child: context.mobile ? _mobileBar(context) : _tabsBar(context, w),
        ),
      ),
      Padding(
        padding: EdgeInsets.only(left: widget.left, top: 6, bottom: 2),
        child: SizedBox(
          width: w,
          child: Stack(children: [
            if (_inlineTitle!.text.isEmpty)
              Positioned.fill(child: IgnorePointer(child: Text('Untitled', style: style.copyWith(color: c.textTertiary)))),
            EditableText(
              key: const ValueKey('db-inline-title'),
              controller: _inlineTitle!,
              focusNode: _titleFocus,
              style: style,
              cursorColor: c.textPrimary,
              backgroundCursorColor: c.textTertiary,
              selectionColor: c.textSelection,
              maxLines: 1,
              selectionControls: noteSelectionControls,
              onChanged: (t) => setState(() {}),
              onSubmitted: (t) => context.app.vault.renamePage(widget.collection, t),
              onTapOutside: (_) {
                if (_titleFocus.hasFocus) {
                  _titleFocus.unfocus();
                  if (_inlineTitle!.text != store.title) context.app.vault.renamePage(widget.collection, _inlineTitle!.text);
                }
              },
            ),
          ]),
        ),
      ),
      if (v != null) _rulesBar(context, v, widget.left),
      _body(context, widget.left),
    ]);
  }

  // ----------------------------------------------------------------- body

  Widget _body(BuildContext context, double gutter) {
    final v = store.view;
    if (!store.loaded || v == null) {
      return widget.inline || widget.blocks ? const SizedBox(height: 120) : const SizedBox.shrink();
    }
    final t = viewRegistry.of(v.kind);
    if (t == null) return const SizedBox.shrink();
    return KeyedSubtree(
      key: ValueKey('view-${v.id}'),
      child: t.build(context, ViewArgs(store: store, view: v, ctl: ctl, inline: widget.inline || widget.blocks, gutter: gutter)),
    );
  }

  // ----------------------------------------------------------------- tabs

  static const _toolbarWidth = 5 * 28.0 + 4 * 2 + 12 + 72;

  double _tabWidth(BuildContext context, ViewDef v) {
    final tp = TextPainter(
      text: TextSpan(text: v.name, style: ts(context.type.ui, context.colors.textPrimary, weight: 500)),
      textDirection: TextDirection.ltr,
      maxLines: 1,
    )..layout(maxWidth: 220);
    return tp.width + 16 + 6 + 20 + 4;
  }

  Widget _tabsBar(BuildContext context, double width) {
    final views = store.views;
    final active = store.view;
    // Room for the tabs: the toolbar, the "+" and, when some tabs do not
    // fit, the "N more…" button.
    final plus = widget.blocks ? 96.0 : 32.0;
    const more = 84.0;
    final avail = width - (_searching ? _toolbarWidth + 150 : _toolbarWidth) - plus - 8;
    final widths = {for (final v in views) v.id: _tabWidth(context, v)};
    final total = widths.values.fold(0.0, (a, b) => a + b);
    var shown = [...views];
    if (total > avail) {
      shown = [];
      var used = 0.0;
      for (final v in views) {
        if (used + widths[v.id]! > avail - more) break;
        shown.add(v);
        used += widths[v.id]!;
      }
      if (active != null && !shown.contains(active)) {
        while (shown.isNotEmpty && used + widths[active.id]! > avail - more) {
          used -= widths[shown.removeLast().id]!;
        }
        shown.add(active);
      }
    }
    final rest = [for (final v in views) if (!shown.contains(v)) v];
    return SizedBox(
      height: 40,
      child: Row(children: [
        Expanded(
          child: SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            physics: const NeverScrollableScrollPhysics(),
            child: Row(children: [
            for (final v in shown) _tab(context, v, v.id == active?.id),
            if (rest.isNotEmpty)
              Builder(
                builder: (b) => Hover(
                  key: const ValueKey('tabs-more'),
                  onTap: () => _moreViews(b, rest),
                  radius: 16,
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                  child: SizedBox(
                    height: 32,
                    child: Center(
                      child: Text(
                              '${rest.length} more…', style: ts(context.type.ui, context.colors.textTertiary, weight: 400)),
                    ),
                  ),
                ),
              ),
            if (widget.blocks)
              Hover(
                key: const ValueKey('add-view'),
                onTap: addListView,
                radius: 16,
                padding: const EdgeInsets.symmetric(horizontal: 8),
                child: SizedBox(
                  height: 32,
                  child: Row(mainAxisSize: MainAxisSize.min, children: [
                    Icon20(Ico.plus, size: 14, color: context.colors.iconTertiary),
                    const SizedBox(width: 4),
                    Text('Add view', style: ts(context.type.ui, context.colors.textTertiary, weight: 400)),
                  ]),
                ),
              )
            else
              Builder(
                builder: (b) => IconBtn(Ico.plus,
                    key: const ValueKey('add-view'),
                    size: 28,
                    glyph: 16,
                    semanticLabel: 'Add a view',
                    color: context.colors.iconTertiary,
                    onTap: () => showAddView(b, store)),
              ),
          ]),
          ),
        ),
        _toolbar(context),
      ]),
    );
  }

  Widget _tab(BuildContext context, ViewDef v, bool active) {
    final c = context.colors;
    final fg = active ? c.textPrimary : c.textSecondary;
    return Padding(
      padding: const EdgeInsets.only(right: 4),
      child: Builder(
        builder: (b) => Hover(
          key: ValueKey('tab-${v.name}'),
          onTap: () => active ? showViewMenu(b, ctl, v) : store.selectView(v.id),
          selected: active,
          radius: 16,
          padding: const EdgeInsets.symmetric(horizontal: 10),
          child: SizedBox(
            height: 32,
            child: Row(mainAxisSize: MainAxisSize.min, children: [
              DbIcon(layoutIcon(v.kind), size: 16, color: active ? c.iconPrimary : c.iconSecondary),
              const SizedBox(width: 6),
              ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 220),
                child: Text(v.name,
                    maxLines: 1, overflow: TextOverflow.ellipsis, style: ts(context.type.ui, fg, weight: active ? 600 : 500)),
              ),
            ]),
          ),
        ),
      ),
    );
  }

  Future<void> _moreViews(BuildContext anchor, List<ViewDef> rest) => showPanel<void>(
        anchor,
        globalRectOf(anchor),
        (ctx) => Column(mainAxisSize: MainAxisSize.min, crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          for (final v in rest)
            dbItem(ctx, v.name, layoutIcon(v.kind), key: ValueKey('more-view-${v.name}'), onTap: () {
              Navigator.of(ctx).pop();
              store.selectView(v.id);
            }),
        ]),
        width: 240,
      );

  // -------------------------------------------------------------- toolbar

  Widget _toolbar(BuildContext context) {
    final c = context.colors;
    final v = store.view;
    final hasFilter = Filters.count(v?.filter) > 0;
    final hasSort = (v?.sorts.isNotEmpty ?? false);
    return Row(mainAxisSize: MainAxisSize.min, children: [
      Builder(
        builder: (b) => DbIconBtn(DbIco.filter,
            key: const ValueKey('db-filter'),
            label: 'Filter',
            color: hasFilter ? c.accent : c.iconTertiary,
            onTap: () => filterButton(b, store)),
      ),
      const SizedBox(width: 2),
      Builder(
        builder: (b) => DbIconBtn(DbIco.sort,
            key: const ValueKey('db-sort'),
            label: 'Sort',
            color: hasSort ? c.accent : c.iconTertiary,
            onTap: () => sortButton(b, store)),
      ),
      const SizedBox(width: 2),
      if (_searching)
        SizedBox(
          width: 150,
          height: 28,
          child: Row(children: [
            Icon20(Ico.search, size: 16, color: c.iconTertiary),
            const SizedBox(width: 6),
            Expanded(
              child: PlainField(
                key: const ValueKey('db-search-field'),
                controller: _search,
                focusNode: _searchFocus,
                autofocus: true,
                placeholder: 'Type to search…',
                style: ts(context.type.ui, c.textPrimary, weight: 400),
                onChanged: store.setSearch,
              ),
            ),
            IconBtn(Ico.close, size: 20, glyph: 12, color: c.iconTertiary, semanticLabel: 'Clear search', onTap: () {
              _search.clear();
              store.setSearch('');
              setState(() => _searching = false);
            }),
          ]),
        )
      else
        IconBtn(Ico.search,
            key: const ValueKey('db-search'),
            size: 28,
            glyph: 16,
            color: c.iconTertiary,
            semanticLabel: 'Search',
            onTap: () => setState(() => _searching = true)),
      const SizedBox(width: 2),
      Builder(
        builder: (b) => DbIconBtn(DbIco.sliders,
            key: const ValueKey('db-settings'), label: 'View settings', color: c.iconTertiary, onTap: () => showViewSettings(b, ctl)),
      ),
      const SizedBox(width: 8),
      _newButton(context),
    ]);
  }

  Widget _newButton(BuildContext context) {
    final c = context.colors;
    final ink = c.onAccent;
    const r =Radii.ctl;
    return ClipRRect(
      borderRadius: r,
      child: ColoredBox(
        color: c.accent,
        child: SizedBox(
          height: 28,
          child: Row(mainAxisSize: MainAxisSize.min, children: [
            Hover(
              key: const ValueKey('db-new'),
              onTap: () => ctl.newRow(context),
              radius: 0,
              hoverColor: c.bgHover,
              padding: const EdgeInsets.symmetric(horizontal: 8),
              child: Center(child: Text('New', style: ts(context.type.ui, ink, weight: 700))),
            ),
            Container(width: 1, height: 28, color: c.onAccent.withValues(alpha: .2)),
            Builder(
              builder: (b) => Hover(
                key: const ValueKey('db-new-menu'),
                onTap: () => showTemplates(b),
                radius: 0,
                hoverColor: c.bgHover,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                child: Center(child: Icon20(Ico.chevronDown, size: 14, color: ink)),
              ),
            ),
          ]),
        ),
      ),
    );
  }

  /// The "New ▾" menu: the templates (features/templates).
  Future<void> showTemplates(BuildContext anchor) => showTemplateMenu(
        anchor,
        parent: store.collection,
        title: store.title,
        defaultId: store.templateId,
        create: (t) async {
          final id = await store.vault.run((v) => v.createPageFrom(parent: store.collection, title: '', template: t), writes: true);
          if (mounted) await ctl.openRow(context, id, isNew: true);
        },
        open: _openFull,
        setDefault: store.setTemplate,
      );

  void _openFull(String id) {
    openPage(context, id, '', stack: true, preserveMobileStack: true);
  }

  // ----------------------------------------------------------- rules bar

  Widget _rulesBar(BuildContext context, ViewDef v, double g) {
    final rules = simpleRulesOf(v);
    final n = Filters.count(v.filter);
    if (n == 0 && v.sorts.isEmpty) return const SizedBox.shrink();
    final c = context.colors;
    return Padding(
      padding: EdgeInsets.fromLTRB(g, 4, g, 4),
      child: SizedBox(
        height: 30,
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Row(children: [
            if (v.sorts.isNotEmpty)
              Builder(
                builder: (b) => ChipBtn(
                  key: const ValueKey('bar-sort'),
                  label: v.sorts.length == 1 ? propName(store, v.sorts.first.prop) : '${v.sorts.length} sorts',
                  icon: DbIco.sort,
                  chevron: true,
                  active: true,
                  onTap: () => showSortEditor(b, globalRectOf(b), store),
                ),
              ),
            if (v.sorts.isNotEmpty && n > 0)
              Container(width: 1, height: 16, margin: const EdgeInsets.symmetric(horizontal: 8), color: c.divider),
            if (rules != null)
              for (var i = 0; i < rules.length; i++)
                Padding(
                  padding: const EdgeInsets.only(right: 6),
                  child: KeyedSubtree(
                    key: ruleChipKey(store, i),
                    child: Builder(
                      builder: (b) => ChipBtn(
                        key: ValueKey('bar-rule-$i'),
                        label: _ruleLabel(rules[i]),
                        icon: propIconOf(store, rules[i].prop),
                        chevron: true,
                        active: true,
                        onTap: () => showRuleEditor(b, globalRectOf(b), store, i),
                      ),
                    ),
                  ),
                )
            else if (n > 0)
              KeyedSubtree(
                key: ruleChipKey(store, -1),
                child: Builder(
                  builder: (b) => ChipBtn(
                    key: const ValueKey('bar-advanced'),
                    label: n == 1 ? '1 rule' : '$n rules',
                    icon: DbIco.filter,
                    chevron: true,
                    active: true,
                    onTap: () => showAdvancedFilter(b, globalRectOf(b), store),
                  ),
                ),
              ),
            Builder(
              builder: (b) => ChipBtn(
                key: const ValueKey('bar-add-filter'),
                label: 'Add filter',
                icon: null,
                onTap: () => filterButton(b, store),
              ),
            ),
          ]),
        ),
      ),
    );
  }

  String _ruleLabel(Condition r) {
    final name = propName(store, r.prop);
    final d = defOf(store, r.prop);
    if (d == null) return name;
    final v = r.value;
    if (v.isEmpty) return name;
    final text = switch (v.kind) {
      ValueKind.select || ValueKind.status => d.option(v.text)?.name ?? '',
      ValueKind.multiSelect => [for (final id in v.ids) d.option(id)?.name ?? ''].join(', '),
      ValueKind.checkbox => v.checked ? 'Checked' : 'Unchecked',
      ValueKind.number => formatNumberValue(v.number, 'number'),
      ValueKind.date => formatDate(v),
      _ => v.text,
    };
    return '$name: $text';
  }

  // --------------------------------------------------------------- mobile

  Widget _mobileBar(BuildContext context) {
    final c = context.colors;
    final v = store.view;
    return SizedBox(
      height: 44,
      child: Row(children: [
        Expanded(
          child: v == null
              ? const SizedBox.shrink()
              : Align(
            alignment: Alignment.centerLeft,
            child: Hover(
              key: const ValueKey('view-switcher'),
              onTap: () => showViewsSheet(context),
              selected: true,
              radius: 18,
              padding: const EdgeInsets.symmetric(horizontal: 12),
              child: SizedBox(
                height: 36,
                child: Row(mainAxisSize: MainAxisSize.min, children: [
                  DbIcon(layoutIcon(v.kind), size: 18, color: c.iconPrimary),
                  const SizedBox(width: 8),
                  Flexible(
                    child: Text(v.name,
                        maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(15, 20, 600, c.textPrimary)),
                  ),
                  const SizedBox(width: 6),
                  Icon20(Ico.chevronDown, size: 16, color: c.iconTertiary),
                ]),
              ),
            ),
          ),
        ),
        IconBtn(Ico.search,
            key: const ValueKey('db-search'),
            size: 40,
            glyph: 20,
            color: c.iconSecondary,
            semanticLabel: 'Search',
            onTap: () => _mobileSearch(context)),
        Builder(
          builder: (b) => DbIconBtn(DbIco.filter,
              key: const ValueKey('db-filter'),
              size: 40,
              glyph: 20,
              label: 'Filter',
              color: Filters.count(v?.filter) > 0 ? c.accent : c.iconSecondary,
              onTap: () => filterButton(b, store)),
        ),
        Builder(
          builder: (b) => DbIconBtn(DbIco.sliders,
              key: const ValueKey('db-settings'),
              size: 40,
              glyph: 20,
              label: 'View settings',
              color: c.iconSecondary,
              onTap: () => showViewSettings(b, ctl)),
        ),
      ]),
    );
  }

  Future<void> _mobileSearch(BuildContext context) => showSheet<void>(
        context,
        (ctx) => SheetSurface(children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(Layout.gutter, 0, Layout.gutter, 12),
            child: SearchField(
              controller: _search,
              focusNode: _searchFocus,
              placeholder: 'Search ${_displayName(ctx)}',
              onChanged: store.setSearch,
            ),
          ),
        ]),
      );

  /// The mobile views sheet: "N views", Done, the views in a card.
  Future<void> showViewsSheet(BuildContext context) => Navigator.of(context).push(_ViewsSheetRoute(this, context.colors.overlay, Motion.of(context).sheet));
}

class _ViewsSheetRoute extends PopupRoute<void> {
  _ViewsSheetRoute(this.db, this.barrier, this.duration);
  final Duration duration;
  final DatabaseViewState db;
  final Color barrier;

  @override
  Color? get barrierColor => barrier;
  @override
  bool get barrierDismissible => true;
  @override
  String? get barrierLabel => 'Close';
  @override
  Duration get transitionDuration => duration;

  @override
  Widget buildTransitions(BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation, Widget child) {
    final t = CurvedAnimation(parent: animation, curve: Motion.easing, reverseCurve: Motion.easing.flipped);
    return SlideTransition(position: Tween(begin: const Offset(0, 1), end: Offset.zero).animate(t), child: child);
  }

  @override
  Widget buildPage(BuildContext context, Animation<double> animation, Animation<double> secondaryAnimation) {
    final mq = MediaQuery.of(context);
    return Align(
      alignment: Alignment.bottomCenter,
      child: ListenableBuilder(
        listenable: db.store,
        builder: (context, _) => _ViewsSheet(db: db, height: mq.size.height - mq.padding.top - 16),
      ),
    );
  }
}

class _ViewsSheet extends StatelessWidget {
  const _ViewsSheet({required this.db, required this.height});
  final DatabaseViewState db;
  final double height;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final store = db.store;
    final views = store.views;
    final active = store.view;
    return Container(
      key: const ValueKey('views-sheet'),
      height: height,
      width: double.infinity,
      decoration: BoxDecoration(
        color: c.bgSecondary,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(Radii.sheet)),
        boxShadow: c.shadowMenu,
      ),
      child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
        const SizedBox(height: 8),
        Center(
          child: Container(
              width: 36, height: 4, decoration: BoxDecoration(color: c.borderStrong, borderRadius: BorderRadius.circular(2))),
        ),
        SizedBox(
          height: 52,
          child: Stack(children: [
            Center(
              child: Text('${views.length} ${views.length == 1 ? 'view' : 'views'}',
                  style: tsRaw(16, 22, 600, c.textPrimary)),
            ),
            Positioned(
              right: 8,
              top: 0,
              bottom: 0,
              child: Center(
                child: TextBtn('Done',
                    key: const ValueKey('views-done'), height: 40, color: c.accent, onTap: () => Navigator.of(context).pop()),
              ),
            ),
          ]),
        ),
        const SizedBox(height: 16),
        Flexible(
          child: SingleChildScrollView(
            padding: const EdgeInsets.symmetric(horizontal: 16),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
              Container(
                decoration: BoxDecoration(color: c.bgPrimary, borderRadius: Radii.panel),
                clipBehavior: Clip.antiAlias,
                child: Column(children: [
                  for (var i = 0; i < views.length; i++) ...[
                    if (i > 0) HLine(color: c.divider),
                    Hover(
                      key: ValueKey('sheet-view-${views[i].name}'),
                      radius: 0,
                      selected: views[i].id == active?.id,
                      onTap: () {
                        store.selectView(views[i].id);
                        Navigator.of(context).pop();
                      },
                      onLongPress: () => showViewMenu(context, db.ctl, views[i]),
                      padding: const EdgeInsets.symmetric(horizontal: 16),
                      child: SizedBox(
                        height: 45,
                        child: Row(children: [
                          DbIcon(layoutIcon(views[i].kind), size: 18, color: c.iconPrimary),
                          const SizedBox(width: 14),
                          Expanded(
                            child: Text(views[i].name,
                                maxLines: 1, overflow: TextOverflow.ellipsis, style: tsRaw(17, 22, 400, c.textPrimary)),
                          ),
                          if (views[i].id == active?.id) Icon20(Ico.check, size: 18, color: c.accent),
                        ]),
                      ),
                    ),
                  ],
                ]),
              ),
              const SizedBox(height: 16),
              Container(
                decoration: BoxDecoration(color: c.bgPrimary, borderRadius: Radii.panel),
                clipBehavior: Clip.antiAlias,
                child: Builder(
                  builder: (b) => Hover(
                    key: const ValueKey('sheet-add-view'),
                    radius: 0,
                    onTap: () {
                      if (db.widget.blocks) {
                        Navigator.of(context).pop();
                        db.addListView();
                      } else {
                        showAddView(b, store);
                      }
                    },
                    padding: const EdgeInsets.symmetric(horizontal: 16),
                    child: SizedBox(
                      height: 45,
                      child: Row(children: [
                        Icon20(Ico.plus, size: 18, color: c.accent),
                        const SizedBox(width: 14),
                        Text('Add view', style: tsRaw(17, 22, 400, c.accentText)),
                      ]),
                    ),
                  ),
                ),
              ),
            ]),
          ),
        ),
      ]),
    );
  }
}
