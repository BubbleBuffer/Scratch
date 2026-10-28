// Desktop sheets share one navigator. The routes retain their editor trees;
// the shell paints the pile's chrome (the spine of the sheet below the top
// one, the paper edges of older sheets and the fan-out list) above them so
// those remain focusable.
//
// The pile has a fixed width: one page deep, the sheet fills the desk with a
// 12 px margin; deeper, only the spine of the sheet directly below shows
// (58 px), with up to three 4 px paper edges for the sheets under it. A hover
// or a click on the pile fans it out as an overlay list.
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../state/app_state.dart';
import '../state/pages_store.dart';
import '../state/tree_store.dart';
import '../ui/ui.dart';

class PageStackNavigator extends NavigatorObserver with ChangeNotifier {
  final List<Route<dynamic>> pages = [];
  final List<Route<dynamic>> _routes = [];
  Route<dynamic>? get currentRoute => _routes.isEmpty ? null : _routes.last;

  /// Every route on the navigator, bottom first.
  List<Route<dynamic>> get routes => List.unmodifiable(_routes);
  final Map<String, String> titles = {};
  final Map<String, ValueChanged<String>> revealBlocks = {};
  final Map<String, ValueChanged<List<String>>> highlightBlocks = {};
  final Map<String, List<String>> _pendingHighlights = {};

  /// Queue a target until its editor registers, without awaiting route close.
  void queueHighlights(String pageId, List<String> blocks) {
    if (blocks.isEmpty) return;
    _pendingHighlights[pageId] = List.unmodifiable(blocks);
    SchedulerBinding.instance.addPostFrameCallback(
      (_) => deliverHighlights(pageId),
    );
  }

  void deliverHighlights(String pageId) {
    if (_disposed) return;
    final callback = highlightBlocks[pageId];
    final blocks = _pendingHighlights[pageId];
    if (callback == null || blocks == null) return;
    _pendingHighlights.remove(pageId);
    callback(blocks);
  }

  /// The first line of body text of each open sheet (for untitled pages).
  final Map<String, String> firstLines = {};
  Route<dynamic>? home;
  bool _disposed = false;
  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }

  String? idOf(Route<dynamic> route) =>
      route.settings.name == '/page' && route.settings.arguments is String
      ? route.settings.arguments as String
      : null;
  Route<dynamic>? routeFor(String id) {
    for (final route in pages) {
      if (idOf(route) == id) return route;
    }
    return null;
  }

  int indexOf(String id) => pages.indexWhere((r) => idOf(r) == id) + 1;
  String? get topId => pages.isEmpty ? null : idOf(pages.last);

  /// Records the first line of body text of open page [id] (empty: none).
  void setFirstLine(String id, String line) {
    if ((firstLines[id] ?? '') == line) return;
    if (line.isEmpty) {
      firstLines.remove(id);
    } else {
      firstLines[id] = line;
    }
    _changed();
  }

  void _changed() {
    if (_disposed) return;
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) {
        if (!_disposed) notifyListeners();
      });
    } else {
      notifyListeners();
    }
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _routes.add(route);
    if (route.isFirst) home = route;
    if (idOf(route) != null) pages.add(route);
    _changed();
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pages.remove(route);
    _routes.remove(route);
    _changed();
  }

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) {
    pages.remove(route);
    _routes.remove(route);
    _changed();
  }

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    final routeIndex = _routes.indexOf(oldRoute!);
    if (routeIndex >= 0) {
      _routes.removeAt(routeIndex);
      if (newRoute != null) _routes.insert(routeIndex, newRoute);
    }
    final index = pages.indexOf(oldRoute);
    if (index >= 0) {
      pages.removeAt(index);
      if (newRoute != null && idOf(newRoute) != null) {
        pages.insert(index, newRoute);
      }
    }
    _changed();
  }

  void popTo(Route<dynamic> route) =>
      navigator?.popUntil((r) => identical(r, route));
  void clearImmediately() {
    for (final route in List<Route<dynamic>>.of(pages).reversed) {
      navigator?.removeRoute(route);
    }
  }
}

class PageStackScope extends InheritedNotifier<PageStackNavigator> {
  const PageStackScope({
    super.key,
    required PageStackNavigator stack,
    required super.child,
  }) : super(notifier: stack);
  static PageStackNavigator of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<PageStackScope>()!.notifier!;
  static PageStackNavigator? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<PageStackScope>()?.notifier;
}

/// Where sheets sit on the desk (x from the rail's right edge).
abstract final class PileGeometry {
  /// The desk margin left of and above every sheet.
  static const margin = 12.0;

  /// The spine of the sheet directly below the top one.
  static const spine = 58.0;

  /// One older sheet's paper edge, and how many of them show at most.
  static const edge = 4.0;
  static const maxEdges = 3;

  /// The paper edges shown in a pile [depth] pages deep (Home not counted).
  static int edges(int depth) => depth < 2 ? 0 : math.min(depth - 2, maxEdges);

  /// The left of the spine in a pile [depth] deep (2 or more).
  static double spineLeft(int depth) => margin + edges(depth) * edge;

  /// The left of the top sheet in a pile [depth] deep.
  static double topLeft(int depth) =>
      depth <= 1 ? margin : spineLeft(depth) + spine;

  /// The left of sheet [index] (1 is the bottom one) in a pile [depth] deep.
  /// The sheet below the top one sits under the spine; older ones step back
  /// one paper edge each, down to the margin.
  static double sheetLeft(int index, int depth) {
    if (index >= depth) return topLeft(depth);
    final below = depth - 1 - index;
    return margin + math.max(0, edges(depth) - below) * edge;
  }
}

/// A page's name where pages are listed: its title, else the first line of
/// its body (live while it is open on the stack, else the stored
/// [firstLine]), else "Untitled".
({String text, bool firstLine}) pageLabel(
  PageStackNavigator? stack,
  String id,
  String title, {
  String firstLine = '',
}) {
  final line = stack?.firstLines[id];
  final bodyLine = line != null && line.trim().isNotEmpty ? line : firstLine;
  return (
    text: displayTitle(title, firstLine: bodyLine),
    firstLine: title.trim().isEmpty && bodyLine.trim().isNotEmpty,
  );
}

/// [pageLabel] with the stack and the stored first line from [context]
/// (an untitled page's label rebuilds when its metadata loads).
({String text, bool firstLine}) pageLabelOf(
  BuildContext context,
  String id,
  String title,
) {
  if (title.trim().isNotEmpty) {
    return (text: displayTitle(title), firstLine: false);
  }
  return pageLabel(
    PageStackScope.maybeOf(context),
    id,
    title,
    firstLine: context.watch<PagesStore>().meta(id)?.firstLine ?? '',
  );
}

/// [style] for a first-line stand-in title: tertiary text in italics.
TextStyle firstLineStyle(TextStyle style, Color tertiary) =>
    style.copyWith(color: tertiary, fontStyle: FontStyle.italic);

/// The label of page [id] as text: an untitled page shows its first line in
/// [tertiary] italics.
class PageLabel extends StatelessWidget {
  const PageLabel({
    super.key,
    required this.id,
    required this.title,
    required this.style,
    this.tertiary,
    this.maxLines = 1,
  });
  final String id, title;
  final TextStyle style;

  /// The colour of a first-line stand-in (textTertiary by default).
  final Color? tertiary;
  final int maxLines;

  @override
  Widget build(BuildContext context) {
    final l = pageLabelOf(context, id, title);
    return Text(
      l.text,
      key: l.firstLine ? ValueKey('first-line-$id') : null,
      maxLines: maxLines,
      overflow: TextOverflow.ellipsis,
      style: l.firstLine
          ? firstLineStyle(style, tertiary ?? context.colors.textTertiary)
          : style,
    );
  }
}

/// Home stays laid out beneath translucent page routes.
class PageStackHome extends StatelessWidget {
  const PageStackHome({super.key, required this.child});
  final Widget child;
  @override
  Widget build(BuildContext context) {
    if (context.mobile) return child;
    final stack = PageStackScope.of(context);
    final covered = stack.pages.isNotEmpty;
    final animation = ModalRoute.of(context)?.secondaryAnimation;
    return SheetCover(covered: covered, animation: animation, child: child);
  }
}

class SheetCover extends StatelessWidget {
  const SheetCover({
    super.key,
    required this.covered,
    required this.child,
    this.animation,
  });
  final bool covered;
  final Widget child;
  final Animation<double>? animation;
  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    Widget surface(double progress) => Stack(
      children: [
        IgnorePointer(
          ignoring: covered || progress > 0,
          child: ExcludeSemantics(excluding: covered, child: child),
        ),
        if (progress > 0)
          Positioned.fill(
            child: IgnorePointer(
              child: ColoredBox(
                color: (c.dark ? const Color(0xFF000000) : c.textPrimary)
                    .withValues(alpha: (c.dark ? .16 : .035) * progress),
              ),
            ),
          ),
      ],
    );
    if (animation == null) return surface(covered ? 1 : 0);
    return AnimatedBuilder(
      animation: animation!,
      builder: (_, _) =>
          surface(covered && animation!.isDismissed ? 1 : animation!.value),
    );
  }
}

SpacePatch? _patchOf(PagesStore pages, String id, AppColors c) {
  final ancestors = pages.ancestors(id);
  return spacePatch(
    ancestors.isEmpty ? pages.meta(id)?.color : ancestors.first.color,
    c.dark ? Brightness.dark : Brightness.light,
  );
}

/// Global spines are outside the inactive routes' FocusScopes.
class PageStackChrome extends StatefulWidget {
  const PageStackChrome({super.key, required this.child});
  final Widget child;

  /// How long the pointer rests on the pile before it fans out.
  static const fanDelay = Duration(milliseconds: 300);

  @override
  State<PageStackChrome> createState() => _PageStackChromeState();
}

class _PageStackChromeState extends State<PageStackChrome> {
  bool _fan = false;
  Timer? _hover;
  FocusNode? _before;
  final _fanFocus = FocusNode(debugLabel: 'stack-fan');

  @override
  void dispose() {
    _hover?.cancel();
    _fanFocus.dispose();
    super.dispose();
  }

  void _openFan() {
    _hover?.cancel();
    if (_fan) return;
    _before = FocusManager.instance.primaryFocus;
    setState(() => _fan = true);
    SchedulerBinding.instance.addPostFrameCallback((_) {
      if (mounted && _fan) _fanFocus.requestFocus();
    });
  }

  void _closeFan() {
    _hover?.cancel();
    if (!_fan) return;
    setState(() => _fan = false);
    final before = _before;
    _before = null;
    if (before != null && before.context != null && before.canRequestFocus) {
      before.requestFocus();
    }
  }

  void _hoverStart() {
    _hover?.cancel();
    _hover = Timer(PageStackChrome.fanDelay, _openFan);
  }

  void _hoverEnd() => _hover?.cancel();

  @override
  Widget build(BuildContext context) {
    if (context.mobile) return widget.child;
    // No store scope before the vault opens: there is no pile to show yet.
    final tree = context.maybeWatch<TreeStore>();
    if (tree == null) return widget.child;
    final stack = PageStackScope.of(context);
    final pages = context.watch<PagesStore>();
    final rail = tree.sidebarCollapsed ? 0.0 : tree.sidebarWidth;
    final c = context.colors;
    final depth = stack.pages.length;
    final showPile =
        depth >= 2 &&
        stack.currentRoute != null &&
        stack.idOf(stack.currentRoute!) != null;
    if (!showPile && _fan) {
      SchedulerBinding.instance.addPostFrameCallback((_) => _closeFan());
    }
    String titleOf(String id) =>
        pages.meta(id)?.title ?? stack.titles[id] ?? '';
    final edges = PileGeometry.edges(depth);
    final spineLeft = rail + PileGeometry.spineLeft(depth);
    final below = depth >= 2 ? stack.pages[depth - 2] : null;
    final belowId = below == null ? null : stack.idOf(below)!;

    return Focus(
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent ||
            event.logicalKey != LogicalKeyboardKey.escape ||
            stack.pages.isEmpty ||
            !stack.pages.last.isCurrent) {
          return KeyEventResult.ignored;
        }
        final focused = FocusManager.instance.primaryFocus?.context;
        if (focused?.widget is EditableText ||
            focused?.findAncestorWidgetOfExactType<EditableText>() != null) {
          return KeyEventResult.ignored;
        }
        stack.navigator?.maybePop();
        return KeyEventResult.handled;
      },
      child: Stack(
        children: [
          Positioned.fill(child: widget.child),
          if (showPile) ...[
            for (var i = 0; i < edges; i++)
              Positioned(
                left: rail + PileGeometry.margin + i * PileGeometry.edge,
                top: PileGeometry.margin,
                bottom: 0,
                width: PileGeometry.edge,
                child: MouseRegion(
                  key: ValueKey('stack-edge-$i'),
                  cursor: SystemMouseCursors.click,
                  onEnter: (_) => _hoverStart(),
                  onExit: (_) => _hoverEnd(),
                  child: Semantics(
                    button: true,
                    label: 'List every open page',
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: _openFan,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: c.bgPrimary,
                          borderRadius: const BorderRadius.only(
                            topLeft: Radius.circular(4),
                          ),
                          border: Border(
                            top: BorderSide(color: c.border),
                            left: BorderSide(color: c.border),
                          ),
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            Positioned(
              left: spineLeft,
              top: PileGeometry.margin,
              bottom: 0,
              width: PileGeometry.spine,
              child: _SpineFade(
                route: below!,
                child: MouseRegion(
                  onEnter: (_) => _hoverStart(),
                  onExit: (_) => _hoverEnd(),
                  child: _Spine(
                    id: belowId!,
                    title: titleOf(belowId),
                    icon: pages.meta(belowId)?.icon,
                    patch: _patchOf(pages, belowId, c),
                    surface: c.bgPrimary,
                    onTap: () {
                      _closeFan();
                      stack.popTo(below);
                    },
                    onFan: _openFan,
                  ),
                ),
              ),
            ),
          ],
          if (_fan && showPile) ...[
            Positioned.fill(
              child: GestureDetector(
                key: const ValueKey('stack-fan-barrier'),
                // Translucent: a tap closes the fan and still reaches the
                // button underneath (the sidebar's feedback button, say).
                behavior: HitTestBehavior.translucent,
                onTap: _closeFan,
              ),
            ),
            Positioned(
              left: spineLeft + PileGeometry.spine + 6,
              top: PileGeometry.margin + 8,
              child: Focus(
                focusNode: _fanFocus,
                onKeyEvent: (_, event) {
                  if (event is KeyDownEvent &&
                      event.logicalKey == LogicalKeyboardKey.escape) {
                    _closeFan();
                    return KeyEventResult.handled;
                  }
                  return KeyEventResult.ignored;
                },
                child: FocusTraversalGroup(
                  child: _FanList(
                    entries: [
                      for (final route in stack.pages.reversed)
                        (
                          route,
                          stack.idOf(route)!,
                          titleOf(stack.idOf(route)!),
                          pages.meta(stack.idOf(route)!)?.icon,
                          _patchOf(pages, stack.idOf(route)!, c),
                        ),
                      if (stack.home != null)
                        (stack.home!, 'home', 'Home', null, null),
                    ],
                    top: stack.pages.last,
                    onPick: (route) {
                      _closeFan();
                      if (!identical(route, stack.pages.last)) {
                        stack.popTo(route);
                      }
                    },
                  ),
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _SpineFade extends StatelessWidget {
  const _SpineFade({required this.route, required this.child});
  final Route<dynamic> route;
  final Widget child;
  @override
  Widget build(BuildContext context) {
    final animation = route is PageRoute
        ? (route as PageRoute).secondaryAnimation
        : null;
    if (animation == null) return child;
    return AnimatedBuilder(
      animation: animation,
      child: child,
      builder: (_, child) => Opacity(
        opacity: animation.isDismissed ? 1 : animation.value,
        child: child,
      ),
    );
  }
}

class _OpenFanIntent extends Intent {
  const _OpenFanIntent();
}

/// The spine of the sheet below the top one: a click pops to it, Enter
/// fans the pile out.
class _Spine extends StatefulWidget {
  const _Spine({
    required this.id,
    required this.title,
    required this.icon,
    required this.surface,
    required this.onTap,
    required this.onFan,
    this.patch,
  });
  final String id, title;
  final String? icon;
  final Color surface;
  final VoidCallback onTap, onFan;
  final SpacePatch? patch;

  @override
  State<_Spine> createState() => _SpineState();
}

class _SpineState extends State<_Spine> {
  bool _hover = false, _focus = false;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final patch = widget.patch;
    final fill = patch?.fill ?? widget.surface;
    const radius = BorderRadius.only(topLeft: Radius.circular(16));
    final label = pageLabel(
      PageStackScope.maybeOf(context),
      widget.id,
      widget.title,
      firstLine: context.watch<PagesStore>().meta(widget.id)?.firstLine ?? '',
    );
    final style = tsDisplay(
      const TypeSpec(15, 18, 640),
      patch?.ink ?? c.textPrimary,
    );
    return Semantics(
      button: true,
      label: 'Back to ${label.text}',
      hint: 'Press Enter to list every open page',
      child: FocusableActionDetector(
        onShowFocusHighlight: (v) => setState(() => _focus = v),
        onShowHoverHighlight: (v) => setState(() => _hover = v),
        mouseCursor: SystemMouseCursors.click,
        shortcuts: const {
          SingleActivator(LogicalKeyboardKey.enter): _OpenFanIntent(),
          SingleActivator(LogicalKeyboardKey.numpadEnter): _OpenFanIntent(),
          SingleActivator(LogicalKeyboardKey.space): ActivateIntent(),
        },
        actions: {
          _OpenFanIntent: CallbackAction<_OpenFanIntent>(
            onInvoke: (_) {
              widget.onFan();
              return null;
            },
          ),
          ActivateIntent: CallbackAction<ActivateIntent>(
            onInvoke: (_) {
              widget.onTap();
              return null;
            },
          ),
        },
        child: GestureDetector(
          key: ValueKey('stack-spine-${widget.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: widget.onTap,
          child: Container(
            decoration: BoxDecoration(
              borderRadius: radius,
              color: _hover
                  ? Color.alphaBlend(
                      (patch?.ink ?? c.accent).withValues(alpha: .12),
                      fill,
                    )
                  : fill,
            ),
            foregroundDecoration: _focus
                ? BoxDecoration(
                    borderRadius: radius,
                    border: Border.all(color: c.focus, width: 2),
                  )
                : null,
            padding: const EdgeInsets.only(top: 18, bottom: 18),
            child: Column(
              children: [
                Container(
                  width: 30,
                  height: 30,
                  alignment: Alignment.center,
                  decoration: BoxDecoration(
                    color: patch?.fill ?? c.bgSecondary,
                    borderRadius: Radii.ctl,
                    border: patch == null ? Border.all(color: c.border) : null,
                  ),
                  child: PageIcon(
                    icon: widget.icon,
                    size: 20,
                    color: patch?.ink ?? c.iconSecondary,
                  ),
                ),
                const SizedBox(height: 12),
                Flexible(
                  child: RotatedBox(
                    quarterTurns: 3,
                    child: Container(
                      // Moss needs its original backing to preserve AA on hover.
                      padding: patch?.fill == const Color(0xFF5F7062)
                          ? const EdgeInsets.symmetric(
                              horizontal: 4,
                              vertical: 2,
                            )
                          : EdgeInsets.zero,
                      decoration: BoxDecoration(
                        color: patch?.fill == const Color(0xFF5F7062)
                            ? patch!.fill
                            : const Color(0x00000000),
                        borderRadius: BorderRadius.circular(4),
                      ),
                      child: Text(
                        label.text,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: label.firstLine
                            ? style.copyWith(fontStyle: FontStyle.italic)
                            : style,
                      ),
                    ),
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// Every sheet of the pile, top first, then Home.
class _FanList extends StatelessWidget {
  const _FanList({
    required this.entries,
    required this.top,
    required this.onPick,
  });
  final List<(Route<dynamic>, String, String, String?, SpacePatch?)> entries;
  final Route<dynamic> top;
  final ValueChanged<Route<dynamic>> onPick;

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final maxH = MediaQuery.sizeOf(context).height - 2 * PileGeometry.margin;
    return ConstrainedBox(
      constraints: BoxConstraints(maxHeight: math.max(120, maxH - 40)),
      child: MenuSurface(
        key: const ValueKey('stack-fan'),
        width: 280,
        padding: const EdgeInsets.all(6),
        child: SingleChildScrollView(
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const MenuHeader('Open pages'),
              for (final (i, e) in entries.indexed)
                Builder(
                  builder: (context) {
                    final label = e.$2 == 'home'
                        ? (text: 'Home', firstLine: false)
                        : pageLabelOf(context, e.$2, e.$3);
                    final style = ts(
                      ty.ui,
                      c.textPrimary,
                      weight: identical(e.$1, top) ? 700 : 400,
                    );
                    return Semantics(
                      button: true,
                      selected: identical(e.$1, top),
                      label: e.$2 == 'home'
                          ? 'Go to Home'
                          : 'Go to ${label.text}',
                      excludeSemantics: true,
                      child: Hover(
                        key: ValueKey('stack-fan-entry-${e.$2}'),
                        onTap: () => onPick(e.$1),
                        selected: identical(e.$1, top),
                        radius: Radii.sm,
                        lift: 0,
                        padding: const EdgeInsets.symmetric(horizontal: 6),
                        child: SizedBox(
                          height: 32,
                          child: Row(
                            children: [
                              if (e.$2 == 'home')
                                SizedBox(
                                  width: 22,
                                  height: 22,
                                  child: Center(
                                    child: Icon20(
                                      Ico.home,
                                      size: 18,
                                      color: c.iconSecondary,
                                    ),
                                  ),
                                )
                              else
                                SpaceIconTile(
                                  icon: e.$4,
                                  patch: e.$5,
                                  size: 22,
                                  glyphSize: 16,
                                  color: c.iconSecondary,
                                ),
                              const SizedBox(width: 8),
                              Expanded(
                                child: Text(
                                  label.text,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: label.firstLine
                                      ? firstLineStyle(style, c.textTertiary)
                                      : style,
                                ),
                              ),
                              if (i == 0)
                                Text(
                                  'On top',
                                  style: ts(
                                    ty.uiSmall,
                                    c.textTertiary,
                                    weight: 400,
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ),
                    );
                  },
                ),
            ],
          ),
        ),
      ),
    );
  }
}
