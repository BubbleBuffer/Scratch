// One route on phones, with a bounded browser-like page history. Covered
// pages are data snapshots, not retained editors or editing sessions.
import 'dart:async';

import 'package:flutter/widgets.dart';

import '../core/models.dart';
import '../editor/page_screen.dart';
import '../state/app_state.dart';
import '../state/page_store.dart';
import '../state/tree_store.dart';
import '../ui/mobile_navigation_gesture.dart';
import '../ui/ui.dart';

class MobilePageEntry {
  MobilePageEntry(this.id, this.title, {this.isNew = false, this.initialBlock});
  final String id;
  final String title;
  bool isNew;
  String? initialBlock;
  PageData? snapshot;
  final Map<String, String> databaseViews = {};
}

/// Small view preferences follow their history entry, including its inert
/// swipe preview. They do not retain a database widget or persist to the vault.
class MobilePageEntryScope extends InheritedWidget {
  const MobilePageEntryScope({
    super.key,
    required this.entry,
    this.preview = false,
    required super.child,
  });
  final MobilePageEntry entry;
  final bool preview;
  static MobilePageEntryScope? maybeOf(BuildContext context) =>
      context.getInheritedWidgetOfExactType<MobilePageEntryScope>();
  @override
  bool updateShouldNotify(MobilePageEntryScope oldWidget) =>
      entry != oldWidget.entry || preview != oldWidget.preview;
}

/// The list contains pages only; stepping before its first entry opens Home.
class MobilePageHistory {
  MobilePageHistory(MobilePageEntry first) : _entries = [first];
  static const limit = 50;
  final List<MobilePageEntry> _entries;
  int index = 0;
  List<MobilePageEntry> get entries => List.unmodifiable(_entries);
  MobilePageEntry get current => _entries[index];
  bool get canForward => index + 1 < _entries.length;
  void append(MobilePageEntry entry) {
    _entries.removeRange(index + 1, _entries.length);
    _entries.add(entry);
    if (_entries.length > limit) _entries.removeAt(0);
    index = _entries.length - 1;
  }
}

class MobilePageHistoryScope extends InheritedWidget {
  const MobilePageHistoryScope({
    super.key,
    required this.host,
    required super.child,
  });
  final MobilePageHistoryState host;
  static MobilePageHistoryState? maybeOf(BuildContext context) =>
      context.getInheritedWidgetOfExactType<MobilePageHistoryScope>()?.host;
  @override
  bool updateShouldNotify(MobilePageHistoryScope oldWidget) => false;
}

class MobilePageHistoryRoute extends PageRouteBuilder<void> {
  MobilePageHistoryRoute(MobilePageEntry entry, MotionDurations motion)
    : history = MobilePageHistory(entry),
      super(
        opaque: false,
        transitionDuration: motion.sheet,
        reverseTransitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox.shrink(),
      );
  final MobilePageHistory history;
  final hostKey = GlobalKey<MobilePageHistoryState>();
  @override
  RouteSettings get settings =>
      RouteSettings(name: '/page', arguments: history.current.id);
  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) => MobilePageHistoryScreen(key: hostKey, route: this);
  @override
  Widget buildTransitions(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
    Widget child,
  ) {
    if (MediaQuery.disableAnimationsOf(context)) return child;
    return SlideTransition(
      position: Tween<Offset>(
        begin: const Offset(1, 0),
        end: Offset.zero,
      ).animate(CurvedAnimation(parent: animation, curve: Motion.easing)),
      child: child,
    );
  }
}

class MobilePageHistoryScreen extends StatefulWidget {
  const MobilePageHistoryScreen({super.key, required this.route});
  final MobilePageHistoryRoute route;
  @override
  State<MobilePageHistoryScreen> createState() => MobilePageHistoryState();
}

class MobilePageHistoryState extends State<MobilePageHistoryScreen>
    with SingleTickerProviderStateMixin {
  MobilePageHistory get history => widget.route.history;
  GlobalKey<PageEditorState> _editorKey = GlobalKey<PageEditorState>();
  late final AnimationController _slide =
      AnimationController.unbounded(vsync: this)..addListener(() {
        if (mounted) setState(() {});
      });
  bool _busy = false;
  bool _leaving = false;
  int _direction = 0;
  double _width = 1;
  double _distance = 0;
  PageEditorState? get editor => _editorKey.currentState;

  void _remember() {
    history.current.snapshot =
        editor?.navigationSnapshot() ?? history.current.snapshot;
    history.current.isNew = false;
    history.current.initialBlock = null;
    editor?.leaveMobileHistory();
  }

  void _publish() {
    context.read<TreeStore>().visit(history.current.id);
    context.app.ui
      ..lastPage = history.current.id
      ..changed();
  }

  Future<void> append(MobilePageEntry entry) async {
    if (_busy || !mounted) return;
    setState(() {
      _busy = true;
      _leaving = true;
    });
    _slide.stop();
    _remember();
    FocusManager.instance.primaryFocus?.unfocus();
    await context.read<PageStore>().lastSave;
    if (!mounted) return;
    setState(() {
      history.append(entry);
      _editorKey = GlobalKey<PageEditorState>();
      _direction = 0;
      _distance = 0;
      _slide.value = 0;
      _busy = false;
      _leaving = false;
    });
    _publish();
  }

  /// Used by the header and Android back as well as a committed right swipe.
  Future<void> back() async {
    if (_busy || !mounted) return;
    if (history.index == 0) {
      _remember();
      Navigator.of(context).pop();
      return;
    }
    await _move(-1);
  }

  Future<void> _move(int delta) async {
    if (_busy || !mounted) return;
    setState(() {
      _busy = true;
      _leaving = true;
    });
    _remember();
    FocusManager.instance.primaryFocus?.unfocus();
    await context.read<PageStore>().lastSave;
    if (!mounted) return;
    setState(() {
      history.index += delta;
      _editorKey = GlobalKey<PageEditorState>();
      _direction = 0;
      _distance = 0;
      _slide.value = 0;
      _busy = false;
      _leaving = false;
    });
    _publish();
  }

  void _start() {
    _slide.stop();
    _distance = 0;
    _direction = 0;
  }

  void _update(DragUpdateDetails details) {
    if (_busy) return;
    _distance += details.delta.dx;
    if (_direction == 0 && _distance.abs() > 0) {
      _direction = _distance > 0 ? 1 : -1;
    }
    if (_direction < 0 && !history.canForward) {
      _slide.value = 0;
      return;
    }
    _slide.value = _direction > 0
        ? _distance.clamp(0, _width).toDouble()
        : _distance.clamp(-_width, 0).toDouble();
  }

  Future<void> _release([DragEndDetails? details]) async {
    if (_busy || _direction == 0) return;
    final velocity = details?.velocity.pixelsPerSecond.dx ?? 0;
    final commit =
        details != null &&
        (_direction > 0 || history.canForward) &&
        (_slide.value.abs() >= _width * .5 || velocity * _direction > 700);
    setState(() {
      _busy = true;
      _leaving = commit;
    });
    if (commit) {
      // Freeze and save before unfocusing/animating: a queued block echo must
      // not replace a still-unflushed title while the sheet slides away.
      _remember();
      FocusManager.instance.primaryFocus?.unfocus();
    }
    final target = commit ? _width * _direction : 0.0;
    if (MediaQuery.disableAnimationsOf(context)) {
      _slide.value = target;
    } else {
      try {
        await _slide
            .animateTo(
              target,
              duration: Motion.of(context).standard,
              curve: Motion.easing,
            )
            .orCancel;
      } on TickerCanceled {
        return;
      }
    }
    if (!mounted) return;
    _busy = false;
    if (commit) {
      if (_direction > 0) {
        await back();
      } else {
        await _move(1);
      }
    } else {
      setState(() {
        _direction = 0;
        _distance = 0;
        _leaving = false;
      });
    }
  }

  @override
  void dispose() {
    _slide.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final entry = history.current;
    return MobilePageHistoryScope(
      host: this,
      child: PopScope<void>(
        canPop: history.index == 0 && !_busy,
        onPopInvokedWithResult: (didPop, _) {
          if (didPop) {
            _remember();
            context.app.ui
              ..lastPage = null
              ..changed();
          } else {
            unawaited(back());
          }
        },
        child: LayoutBuilder(
          builder: (context, box) {
            _width = box.maxWidth;
            final neighborIndex = history.index + (_direction > 0 ? -1 : 1);
            final snapshot =
                _direction != 0 &&
                    neighborIndex >= 0 &&
                    neighborIndex < history.entries.length
                ? history.entries[neighborIndex].snapshot
                : null;
            return MobileNavigationGesture(
              enabled: context.mobile && !_busy,
              canStart: () => !(editor?.navigationBlocked ?? true),
              onStart: _start,
              onUpdate: _update,
              onEnd: (details) => unawaited(_release(details)),
              onCancel: () => unawaited(_release()),
              child: ClipRect(
                child: Stack(
                  children: [
                    if (snapshot != null)
                      Positioned.fill(
                        child: Transform.translate(
                          offset: Offset(
                            -_direction * _width * .25 + _slide.value * .25,
                            0,
                          ),
                          child: IgnorePointer(
                            child: ExcludeFocus(
                              child: ExcludeSemantics(
                                child: MobilePageEntryScope(
                                  entry: history.entries[neighborIndex],
                                  preview: true,
                                  child: PageScreen(
                                    key: ValueKey(
                                      'mobile-history-preview-$neighborIndex',
                                    ),
                                    pageId: snapshot.id,
                                    initialTitle: snapshot.title,
                                    snapshot: snapshot,
                                    navigationPreview: true,
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                    Positioned.fill(
                      child: Transform.translate(
                        key: const ValueKey('mobile-history-sheet'),
                        offset: Offset(_slide.value, 0),
                        child: IgnorePointer(
                          ignoring: _busy,
                          child: ExcludeFocus(
                            excluding: _leaving,
                            child: DecoratedBox(
                              decoration: BoxDecoration(
                                boxShadow: context.colors.stack,
                              ),
                              child: MobilePageEntryScope(
                                entry: entry,
                                child: PageScreen(
                                  key: _editorKey,
                                  pageId: entry.id,
                                  initialTitle: entry.title,
                                  isNew: entry.isNew,
                                  initialBlock: entry.initialBlock,
                                ),
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
          },
        ),
      ),
    );
  }
}
