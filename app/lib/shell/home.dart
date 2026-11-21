// Today shows the selected day and its editable Journal note. The shell keeps
// its existing navigation, quick capture and searchable page disclosure.

import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../core/paths.dart';
import '../registry/addon_registry.dart';
import 'addons.dart';
import '../features/bakeoff.dart';
import '../features/today_view.dart';
import '../features/today_day_note.dart';
import '../features/today_date_strip.dart';
import '../features/settings/settings_screen.dart';
import '../features/sync/conflicts.dart' show ConflictHomeSection;
import '../features/sync/sync_indicator.dart';
import '../features/trash/trash_panel.dart';
import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../state/reminder_store.dart';
import '../state/tree_store.dart';
import '../ui/calico_art.dart';
import '../ui/icon_x.dart';
import '../ui/ui.dart';
import '../ui/toast.dart';
import 'mobile_home_panels.dart';
import 'navigation.dart';
import 'page_links.dart';
import 'search.dart';
import 'sidebar.dart';
import 'today.dart';

// ---------------------------------------------------------------------------
// Home

class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key, this.startedAtUs, this.restoreLastPage = false});
  final int? startedAtUs;

  /// Reopen the page that was open when the app last ran (at start).
  final bool restoreLastPage;

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  final _search = TextEditingController();
  final _searchFocus = FocusNode();
  final _searchKey = GlobalKey();
  final _scroll = ScrollController();
  final _listKey = GlobalKey();
  final _firstRowKey = GlobalKey();
  late final PageSearch _query = PageSearch(context.app.vault)
    ..addListener(_changed);
  late final PagesStore _pages = context.read<PagesStore>()
    ..addListener(_pagesChanged);
  late final ReminderStore _reminders = context.read<ReminderStore>();
  late final PageLinkListener _pageLinks = PageLinkListener(
    (link) => openPageLink(context, link),
  );
  bool _layoutLogged = false;

  /// Mobile: the side panels (page tree and agent chat) beside Home.
  final _panels = GlobalKey<MobileHomePanelsState>();

  @override
  void initState() {
    super.initState();
    BootTiming.mark('home built');
    searchRequest.addListener(_focusSearch);
    _mobilePagesRequest.addListener(_showPages);
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      BootTiming.mark('home first frame');
      final s = widget.startedAtUs;
      final ms = s == null
          ? -1
          : (clockNow().microsecondsSinceEpoch - s) / 1000.0;
      Bakeoff.instance.log(
        'list-visible ms-since-main=${ms.toStringAsFixed(0)} pages=${_pages.count}',
      );
      final reminders = _reminders;
      final launchTap = reminders.hasPendingTap;
      reminders.bind(
        taps: (payload) => openReminder(context, payload),
        showToast: (title, body) => showErrorToast(context, '$title\n$body'),
      );
      final restore = widget.restoreLastPage && !launchTap;
      if (_pageLinks.enabled) {
        unawaited(_startPageLinks(restore: restore));
      } else if (restore) {
        _restoreLastPage();
      }
    });
  }

  /// Opens the page link that started the app, if there is one; it replaces
  /// the restored page. Otherwise the last page is restored as before.
  Future<void> _startPageLinks({required bool restore}) async {
    final initial = await _pageLinks.start();
    if (!mounted) return;
    if (initial != null) {
      await openPageLink(context, initial);
    } else if (restore) {
      await _restoreLastPage();
    }
  }

  /// Opens the page that was open at the last run, if it still exists.
  Future<void> _restoreLastPage() async {
    final ui = context.app.ui;
    final id = ui.lastPage;
    if (id == null) return;
    try {
      BootTiming.mark('restore last page: loadPage begin');
      final p = await context.app.vault.loadPage(id);
      BootTiming.mark('restore last page: loadPage done');
      if (!mounted) return;
      if (p.inTrash) throw StateError('in the trash');
      await openPage(context, id, p.title);
      BootTiming.mark('restore last page: openPage done');
    } catch (_) {
      if (ui.lastPage == id) {
        ui.lastPage = null;
        ui.changed();
      }
    }
  }

  @override
  void dispose() {
    _mobilePagesRequest.removeListener(_showPages);
    _reminders.unbind();
    _pageLinks.dispose();
    _pages.removeListener(_pagesChanged);
    _query.dispose();
    searchRequest.removeListener(_focusSearch);
    _search.dispose();
    _searchFocus.dispose();
    _scroll.dispose();
    super.dispose();
  }

  void _changed() => setState(() {});

  void _showPages() => _panels.currentState?.openTree();

  void _pagesChanged() {
    if (_query.hits != null) _query.refresh();
  }

  void _focusSearch() {
    if (!mounted) return;
    if (context.sizeClass.isCompact) {
      _openSearch();
    } else {
      final previousEnd = _scroll.hasClients
          ? _scroll.position.maxScrollExtent
          : 0.0;
      setState(() {
        context.app.ui
          ..allPagesExpanded = true
          ..changed();
      });
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!mounted) return;
        if (_scroll.hasClients) {
          _scroll.jumpTo(
            previousEnd.clamp(0.0, _scroll.position.maxScrollExtent),
          );
        }
        SchedulerBinding.instance.addPostFrameCallback((_) {
          if (!mounted) return;
          final field = _searchKey.currentContext;
          if (field != null) Scrollable.ensureVisible(field, alignment: 0.2);
          _searchFocus.requestFocus();
        });
      });
    }
  }

  void _openSearch() =>
      pushScreen<void>(context, (_) => const SearchScreen(), name: '/search');

  List<Hit>? get _hits => _query.hits;

  Future<void> _open(String id, String title) => openPage(context, id, title);

  void _logLayout(MediaQueryData mq, PagesStore pages) {
    if (_layoutLogged || pages.count == 0 || !Bakeoff.enabled) return;
    _layoutLogged = true;
    // Tap targets for measure.ps1, in physical pixels.
    SchedulerBinding.instance.addPostFrameCallback((_) {
      final list = _listKey.currentContext?.findRenderObject() as RenderBox?;
      final row = _firstRowKey.currentContext?.findRenderObject() as RenderBox?;
      if (list == null || row == null) return;
      final dpr = mq.devicePixelRatio;
      final lt = list.localToGlobal(Offset.zero);
      final rt = row.localToGlobal(Offset.zero);
      var big = -1;
      for (var i = 0; i < 20; i++) {
        if (pages.rowAt(i)?.title == 'Big page') {
          big = i;
          break;
        }
      }
      Bakeoff.instance.log(
        'layout dpr=$dpr first-row-x=${((rt.dx + row.size.width / 2) * dpr).round()} '
        'first-row-y=${((rt.dy + row.size.height / 2) * dpr).round()} row-h=${(row.size.height * dpr).round()} '
        'big-index=$big '
        'list-top=${(lt.dy * dpr).round()} list-bottom=${((lt.dy + list.size.height) * dpr).round()} '
        'screen-w=${(mq.size.width * dpr).round()} screen-h=${(mq.size.height * dpr).round()}',
      );
    });
  }

  @override
  Widget build(BuildContext context) {
    final mq = MediaQuery.of(context);
    final pages = context.watch<PagesStore>();
    _logLayout(mq, pages);
    return context.sizeClass.isCompact ? _mobile(context, mq) : _desktop(context);
  }

  // ---- shared pieces ----

  Widget _searchField() => SearchField(
    key: _searchKey,
    fieldKey: const ValueKey('search-field'),
    controller: _search,
    focusNode: _searchFocus,
    onChanged: _query.run,
  );

  /// Rows of the list: search results, or every page except database rows
  /// (one more while the scan for them goes on).
  int get _count =>
      _hits?.length ??
      (_pages.docComplete ? _pages.docCount : _pages.docCount + 1);

  /// The "All pages" count, once known.
  String? get _countLabel {
    final h = _hits;
    if (h != null) return formatCount(h.length);
    return _pages.docComplete ? formatCount(_pages.docCount) : null;
  }

  Widget _row(BuildContext context, int i) {
    final c = context.colors;
    final ty = context.type;
    final r = _pages.docAt(i);
    final id = r?.id;
    final title = r?.title;
    final sub = r == null ? null : relativeTime(r.updatedMs.toInt());
    final empty = title == null || title.isEmpty;
    final label = id == null ? null : pageLabelOf(context, id, title ?? '');
    final style = ts(ty.item, empty ? c.textSecondary : c.textPrimary);
    final content = Row(
      children: [
        PageIcon(icon: r?.icon, size: context.metrics.icon.px, color: c.iconSecondary),
        Gap(context.metrics.iconGap),
        Expanded(
          child: Text(
            label?.text ?? '',
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: label?.firstLine ?? false
                ? firstLineStyle(style, c.textTertiary)
                : style,
          ),
        ),
        if (sub != null) ...[
          const Gap(Space.md),
          Text(sub, style: ts(ty.uiSmall, c.textTertiary, weight: Weight.regular)),
        ],
      ],
    );
    return Hover(
      key: i == 0 ? _firstRowKey : (id == null ? null : ValueKey('row-$id')),
      onTap: id == null ? null : () => _open(id, title ?? ''),
      borderRadius: Radii.ctl,
      padding: EdgeInsets.symmetric(horizontal: context.metrics.insetX),
      child: Semantics(button: id != null, child: content),
    );
  }

  Widget _list({required double gutter}) {
    final hits = _hits;
    return SliverPadding(
      padding: EdgeInsets.symmetric(horizontal: gutter),
      sliver: SliverFixedExtentList(
        itemExtent: hits != null
            ? SearchResultRow.extent(context.metrics)
            : context.metrics.listRow.height,
        delegate: SliverChildBuilderDelegate(
          (context, i) => Padding(
            padding: EdgeInsets.symmetric(vertical: context.touch ? 0 : Space.hair),
            child: hits != null
                ? SearchResultRow(
                    hit: hits[i],
                    onTap: () => _open(hits[i].id, hits[i].title),
                  )
                : _row(context, i),
          ),
          childCount: _count,
        ),
      ),
    );
  }

  NotificationListener<ScrollNotification> _scrollView(List<Widget> slivers) =>
      NotificationListener(
        onNotification: (s) {
          if (s.metrics.axis != Axis.vertical) return false;
          if (s is ScrollStartNotification) {
            Bakeoff.instance.scrollStarted('list');
          }
          if (s is ScrollEndNotification) Bakeoff.instance.scrollEnded();
          return false;
        },
        child: CustomScrollView(
          key: _listKey,
          controller: _scroll,
          slivers: slivers,
        ),
      );

  Widget _organizer(BuildContext context) => TodayView(
    includeStrip: false,
    note: Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        const TodayDayNote(),
        const Gap(Space.xl),
        TodayCapture(mobile: context.sizeClass.isCompact),
      ],
    ),
  );

  List<Widget> _allPages(
    BuildContext context,
    double gutter,
  ) {
    final c = context.colors;
    final expanded = context.app.ui.allPagesExpanded;
    // The header count needs the scan too; boot no longer runs it.
    if (!expanded) _pages.startDocScan();
    return [
      SliverToBoxAdapter(
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            gutter + context.metrics.insetX,
            Space.xl,
            gutter + context.metrics.insetX,
            Space.sm,
          ),
          child: Hover(
            key: const ValueKey('all-pages-toggle'),
            onTap: () => setState(() {
              context.app.ui
                ..allPagesExpanded = !expanded
                ..changed();
            }),
            padding: const EdgeInsets.symmetric(vertical: Space.sm),
            child: Row(
              children: [
                Icon20(
                  expanded ? Ico.chevronDown : Ico.chevronRight,
                  size: IconSize.xs,
                  color: c.iconTertiary,
                ),
                const Gap(Space.sm),
                Text(
                  'All pages${_countLabel == null ? '' : ' · $_countLabel'}',
                  style: ts(context.type.ui, c.textSecondary),
                ),
              ],
            ),
          ),
        ),
      ),
      if (expanded) ...[
        SliverToBoxAdapter(
          child: Padding(
            padding: EdgeInsets.fromLTRB(
              gutter + context.metrics.insetX,
              0,
              gutter + context.metrics.insetX,
              Space.sm,
            ),
            child: _searchField(),
          ),
        ),
        _list(gutter: gutter),
      ],
    ];
  }

  // ---- desktop ----
  Widget _desktop(BuildContext context) {
    final c = context.colors;
    return ColoredBox(
      color: c.desk,
      child: Row(
        children: [
          const Sidebar(),
          Expanded(
            child: Column(
              children: [
                SizedBox(
                  height: Layout.topbarHeight,
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: Space.md),
                    child: Row(
                      children: [
                        const SidebarReopenButton(),
                        const CollapsedFeedbackButton(),
                        Icon20(Ico.home, size: IconSize.sm, color: c.iconSecondary),
                        const Gap(Space.xs),
                        Text(
                          'Today',
                          style: ts(
                            context.type.ui,
                            c.textPrimary,
                            weight: Weight.regular,
                          ),
                        ),
                        const Spacer(),
                        const SyncIndicator(),
                      ],
                    ),
                  ),
                ),
                const Padding(
                  padding: EdgeInsets.symmetric(horizontal: Space.xl),
                  child: TodayDateStrip(),
                ),
                Expanded(
                  child: LayoutBuilder(
                    builder: (context, box) {
                      final width = math.min(box.maxWidth, HomeLayout.width);
                      final gutter = (box.maxWidth - width) / 2 + HomeLayout.margin;
                      return _scrollView([
                        SliverToBoxAdapter(
                          child: Padding(
                            padding: EdgeInsets.fromLTRB(gutter, Space.xxl, gutter, 0),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.stretch,
                              children: [_organizer(context)],
                            ),
                          ),
                        ),
                        ..._allPages(context, gutter - context.metrics.insetX),
                        const SliverToBoxAdapter(child: SizedBox(height: HomeLayout.endPadding)),
                      ]);
                    },
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  // ---- mobile ----
  /// Home is the only root. The page tree and the agent chat slide in beside
  /// it, and the dock at the bottom opens the tree.
  Widget _mobile(BuildContext context, MediaQueryData mq) {
    final c = context.colors;
    return ColoredBox(
      color: c.desk,
      child: MobileHomePanels(
        key: _panels,
        tree: const ShellMobileFrame(
          destinationId: 'pages',
          showDock: false,
          child: MobilePages(key: ValueKey('mobile-pages')),
        ),
        hasAgent: appAddonContext.shellPanels.all.isNotEmpty,
        agent:
            appAddonContext.shellPanels.all.firstOrNull?.build(context) ??
            const SizedBox.shrink(),
        home: Padding(
          padding: EdgeInsets.only(
            top: mq.padding.top,
            bottom: mq.viewInsets.bottom,
          ),
          child: ShellMobileFrame(
            destinationId: 'today',
            onPages: () => _panels.currentState?.openTree(),
            onToday: () => _panels.currentState?.close(),
            child: Column(
              children: [
                const Padding(
                  padding: EdgeInsets.fromLTRB(Space.md, Space.xxs, Space.md, 0),
                  child: TodayDateStrip(),
                ),
                Expanded(child: _mobileHome(context)),
              ],
            ),
          ),
        ),
      ),
    );
  }

  Widget _mobileHome(BuildContext context) {
    final c = context.colors;
    return _scrollView([
      SliverToBoxAdapter(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(Space.lg, Space.md, Space.sm, 0),
          child: Row(
            children: [
              const CalicoMark(key: ValueKey('home-mark'), size: ArtSizes.calicoMarkSmall),
              const Gap(Space.sm),
              Expanded(
                child: Text(
                  'Today',
                  style: ts(context.type.ui, c.textPrimary, weight: Weight.strong),
                ),
              ),
              const SizedBox(height: Layout.minTapTarget, child: Center(child: SyncIndicator())),
              const Gap(Space.md),
            ],
          ),
        ),
      ),
      const SliverToBoxAdapter(child: ConflictHomeSection()),
      SliverToBoxAdapter(
        child: Padding(
          padding: const EdgeInsets.fromLTRB(Space.lg, Space.lg, Space.lg, 0),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [_organizer(context)],
          ),
        ),
      ),
      ..._allPages(context, 0),
      const SliverToBoxAdapter(child: SizedBox(height: HomeLayout.compactEndPadding)),
    ]);
  }
}

final _mobilePagesRequest = ValueNotifier<int>(0);

List<AddonShellNavigation> _dockDestinations() {
  final entries = (activeAddonContext ?? appAddonContext).shellNavigation.all
      .where((entry) => entry.mobileDock)
      .toList();
  entries.sort(
    (a, b) =>
        a.order != b.order ? a.order.compareTo(b.order) : a.id.compareTo(b.id),
  );
  return entries;
}

/// Shared phone navigation and create action for a top-level shell view.
/// Add-ons supply destinations and actions through the registry.
class ShellMobileFrame extends StatelessWidget {
  const ShellMobileFrame({
    super.key,
    required this.destinationId,
    required this.child,
    this.onPages,
    this.onToday,
    this.showDock = true,
  });
  final String destinationId;
  final Widget child;
  final VoidCallback? onPages, onToday;
  final bool showDock;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final actions =
        (activeAddonContext ?? appAddonContext).shellPrimaryActions.all;
    final action = actions
        .where((item) => item.destinations.contains(destinationId))
        .firstOrNull;
    final pageAction = destinationId == 'pages';
    return Column(
      children: [
        Expanded(
          child: Stack(
            children: [
              Positioned.fill(child: child),
              if (pageAction || action != null)
                Positioned(
                  right: Space.lg,
                  bottom:
                      Space.lg +
                      (showDock ? 0 : MediaQuery.paddingOf(context).bottom),
                  child: Semantics(
                    button: true,
                    label: pageAction ? 'New page' : action!.label,
                    child: AppTooltip(
                      label: pageAction ? 'New page' : action!.label,
                      command: pageAction ? 'page.new' : null,
                      child: Hover(
                        key: ValueKey(pageAction ? 'new-page' : 'mobile-create'),
                        onTap: pageAction
                            ? () => createPage(context)
                            : () => action!.run(context),
                        background: c.accent,
                        hoverBackground: c.accentHover,
                        pressedBackground: c.accentHover,
                        borderRadius: Radii.panel,
                        shadow: c.stack,
                        child: SizedBox(
                          width: ControlSize.xxl.height,
                          height: ControlSize.xxl.height,
                          child: Center(
                            child: Icon20(Ico.plus, size: IconSize.xl, color: c.accentInk),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
            ],
          ),
        ),
        if (showDock)
          _MobileDock(
            destinationId: destinationId,
            onPages:
                onPages ??
                () {
                  _mobilePagesRequest.value++;
                  goHome(context);
                },
            onToday: onToday ?? () => goHome(context),
            onSearch: () => pushScreen<void>(
              context,
              (_) => const SearchScreen(),
              name: '/search',
            ),
          ),
      ],
    );
  }
}

class _MobileDock extends StatelessWidget {
  const _MobileDock({
    required this.destinationId,
    required this.onPages,
    required this.onToday,
    required this.onSearch,
  });
  final String destinationId;
  final VoidCallback onPages, onToday, onSearch;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final mq = MediaQuery.of(context);
    Widget button(String id, Ico icon, String label, VoidCallback onTap) =>
        Semantics(
          button: true,
          selected: destinationId == id,
          label: label,
          child: Hover(
            key: ValueKey('nav-$id'),
            onTap: onTap,
            borderRadius: Radii.ctl,
            child: ConstrainedBox(
              constraints: BoxConstraints(minWidth: MobileDock.destination, minHeight: ControlSize.xl.height),
              child: Column(
                mainAxisAlignment: MainAxisAlignment.center,
                children: [
                  Icon20(
                    icon,
                    size: IconSize.lg,
                    color: destinationId == id ? c.accentText : c.iconSecondary,
                  ),
                  Text(
                    label,
                    style: ts(context.type.caption, destinationId == id ? c.accentText : c.textSecondary, weight: Weight.strong),
                  ),
                ],
              ),
            ),
          ),
        );
    return Container(
      key: const ValueKey('mobile-dock'),
      padding: EdgeInsets.fromLTRB(Space.md, Space.sm, Space.md, mq.padding.bottom + Space.sm),
      decoration: BoxDecoration(
        color: c.bgPrimary,
        border: Border(top: BorderSide(color: c.border)),
      ),
      child: Row(
        children: [
          button('pages', Ico.sidebar, 'Pages', onPages),
          button('today', Ico.home, 'Today', onToday),
          for (final entry in _dockDestinations())
            button(
              entry.id,
              entry.icon,
              entry.label,
              () => entry.open(context),
            ),
          const Gap(Space.xs),
          Expanded(
            child: Semantics(
              button: true,
              label: 'Search',
              child: Hover(
                key: const ValueKey('nav-search'),
                onTap: onSearch,
                borderRadius: Radii.pill,
                child: Container(
                  constraints: BoxConstraints(minHeight: ControlSize.xl.height),
                  padding: const EdgeInsets.symmetric(horizontal: Space.md),
                  decoration: BoxDecoration(
                    color: c.bgSecondary,
                    borderRadius: Radii.pill,
                    border: Border.all(color: c.border),
                  ),
                  child: Row(
                    children: [
                      Icon20(Ico.search, size: IconSize.md, color: c.iconTertiary),
                      const Gap(Space.xs),
                      Expanded(
                        child: Text(
                          'Search',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: ts(context.type.uiSmall, c.textTertiary),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

// ---------------------------------------------------------------------------
// Mobile Pages tab

/// The mobile Pages tab: the sidebar's page tree, from the same
/// [TreeStore] (expansion included), as a full-screen list with large rows,
/// then Trash and Settings.
class MobilePages extends StatelessWidget {
  const MobilePages({super.key});

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tree = context.watch<TreeStore>();
    final top = tree.childrenOf(null);
    final rows = tree.visibleRows();
    final favorites = tree.favorites;
    return ColoredBox(
      color: c.desk,
      child: ListView(
        key: const ValueKey('pages-list'),
        padding: const EdgeInsets.only(top: Space.md, bottom: HomeLayout.compactEndPadding),
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(Space.lg, Space.md, Space.lg, Space.md),
            child: Text(
              'Pages',
              style: tsDisplay(context.type.h1, c.textPrimary, weight: Weight.display),
            ),
          ),
          if (favorites.isNotEmpty) ...[
            const _PagesSection('Favorites'),
            for (final r in tree.favoriteRows())
              _PageTreeTile(row: r, fav: true),
            const Gap(Space.sm),
            const _PagesSection('Pages'),
          ],
          for (final r in rows) _PageTreeTile(row: r),
          if (top != null && top.isEmpty)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: Space.lg, vertical: Space.md),
              child: Text(
                'No pages yet.',
                style: ts(context.type.body, c.textSecondary),
              ),
            ),
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: Space.lg, vertical: Space.md),
            child: Container(height: Stroke.hair, color: c.border),
          ),
          _PagesAction(
            key: const ValueKey('pages-trash'),
            icon: Ico.trash,
            label: 'Trash',
            onTap: () => showTrash(context),
          ),
          _PagesAction(
            key: const ValueKey('pages-settings'),
            icon: Ico.settings,
            label: 'Settings',
            onTap: () => showSettings(context),
          ),
          _PagesAction(
            key: const ValueKey('pages-feedback'),
            iconX: Ix.feedback,
            label: 'Send feedback',
            onTap: () => sendFeedback(context),
          ),
        ],
      ),
    );
  }
}

class _PagesSection extends StatelessWidget {
  const _PagesSection(this.label);
  final String label;
  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.fromLTRB(Space.lg, Space.sm, Space.lg, Space.xxs),
    child: Text(label, style: ts(context.type.ui, context.colors.textSecondary, weight: Weight.strong)),
  );
}

/// One page in the mobile tree: at least 52 px tall, with a 44 px
/// expand/collapse target for pages that have pages inside.
class _PageTreeTile extends StatelessWidget {
  const _PageTreeTile({required this.row, this.fav = false});
  final TreeRow row;
  final bool fav;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final tree = context.watch<TreeStore>();
    final p = row.page;
    final indent = Space.sm + MobileTree.indent * row.depth;
    final scope = fav ? 'pages-fav-${row.root}-' : 'pages-';
    if (row.placeholder) {
      return Padding(
        key: ValueKey('${scope}empty-${p.id}'),
        padding: EdgeInsets.only(left: indent + Layout.minTapTarget, right: Space.lg),
        child: SizedBox(
          height: MobileTree.emptyRow,
          child: Align(
            alignment: Alignment.centerLeft,
            child: Text(
              'No pages inside',
              style: ts(context.type.ui, c.textTertiary, weight: Weight.regular),
            ),
          ),
        ),
      );
    }
    final pages = context.watch<PagesStore>();
    final ancestors = pages.ancestors(p.id);
    final patch = p.parent == null
        ? spacePatch(
            ancestors.isEmpty ? pages.meta(p.id)?.color : ancestors.first.color,
            c.dark ? Brightness.dark : Brightness.light,
          )
        : null;
    final title = displayTitle(
      p.title,
      firstLine: pages.meta(p.id)?.firstLine ?? '',
    );
    final expanded = tree.isExpanded(p.id, fav: fav);
    final canExpand = p.childCount > 0 || expanded;
    return Hover(
      key: ValueKey('$scope${p.id}'),
      onTap: () => openPage(context, p.id, p.title),
      borderRadius: Radii.ctl,
      padding: EdgeInsets.only(left: indent, right: Space.lg),
      child: ConstrainedBox(
        constraints: const BoxConstraints(minHeight: MobileTree.row),
        child: Row(
          children: [
            if (canExpand)
              Semantics(
                button: true,
                expanded: expanded,
                label: expanded ? 'Collapse $title' : 'Expand $title',
                excludeSemantics: true,
                child: GestureDetector(
                  key: ValueKey('${scope}toggle-${p.id}'),
                  behavior: HitTestBehavior.opaque,
                  onTap: () => tree.toggle(p.id, fav: fav),
                  child: SizedBox(
                    width: Layout.minTapTarget,
                    height: MobileTree.row,
                    child: Center(
                      child: Icon20(
                        expanded ? Ico.chevronDown : Ico.chevronRight,
                        size: IconSize.sm,
                        color: c.iconSecondary,
                      ),
                    ),
                  ),
                ),
              )
            else
              const Gap(Layout.minTapTarget),
            SpaceIconTile(
              icon: p.icon,
              patch: patch,
              size: MobileTree.tile,
              glyphSize: MobileTree.tileGlyph,
              color: c.iconSecondary,
            ),
            const Gap(Space.md),
            Expanded(
              child: PageLabel(
                id: p.id,
                title: p.title,
                style: ts(
                  context.type.body,
                  p.title.isEmpty ? c.textSecondary : c.textPrimary,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _PagesAction extends StatelessWidget {
  const _PagesAction({
    super.key,
    this.icon,
    this.iconX,
    required this.label,
    required this.onTap,
  }) : assert((icon == null) != (iconX == null));
  final Ico? icon;
  final Ix? iconX;
  final String label;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    return Semantics(
      button: true,
      child: Hover(
        onTap: onTap,
        borderRadius: Radii.ctl,
        // Icons line up with the page icons above.
        padding: const EdgeInsets.only(left: Space.sm + Layout.minTapTarget, right: Space.lg),
        child: ConstrainedBox(
          constraints: const BoxConstraints(minHeight: MobileTree.row),
          child: Row(
            children: [
              SizedBox(
                width: MobileTree.tile,
                child: Center(
                  child: icon != null
                      ? Icon20(icon!, size: IconSize.xl, color: c.iconSecondary)
                      : IconX(iconX!, size: IconSize.xl.px, color: c.iconSecondary),
                ),
              ),
              const Gap(Space.md),
              Expanded(
                child: Text(label, style: ts(context.type.body, c.textPrimary)),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
