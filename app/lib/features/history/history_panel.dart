// Version history of a page: the versions (list_versions, newest first),
// a read-only preview of the selected one (load_page_at, drawn as plain
// styled text, not the editor) and "Restore version" (restore_version),
// which closes the panel. Desktop: a large modal, list on the left.
// Mobile: a full-screen page, list on top.

import 'package:flutter/widgets.dart';

import '../../core/models.dart';
import '../../state/app_state.dart';
import '../../editor/page_screen.dart';
import '../../ui/ui.dart';

/// The local time of a version (overridden by golden tests, whose versions
/// are made at the real time).
DateTime Function(int ms) versionTimeOf = DateTime.fromMillisecondsSinceEpoch;

/// Shows page [pageId]'s version history.
Future<void> showVersionHistory(BuildContext context, String pageId) async {
  if (context.mobile) {
    await pushScreen<void>(context, (_) => HistoryPanel(key: const ValueKey('history-panel'), pageId: pageId),
        name: '/history');
    return;
  }
  await showModalPanel<void>(context, (_) => HistoryPanel(key: const ValueKey('history-panel'), pageId: pageId),
      width: 1100, height: 760);
}

class HistoryPanel extends StatefulWidget {
  const HistoryPanel({super.key, required this.pageId});
  final String pageId;

  @override
  State<HistoryPanel> createState() => _HistoryPanelState();
}

class _HistoryPanelState extends State<HistoryPanel> {
  List<VersionInfo>? _versions;
  int _selected = 0;
  PageData? _preview;
  String? _error;
  bool _restoring = false;
  int _gen = 0;

  @override
  void initState() {
    super.initState();
    _load();
  }

  Future<void> _load() async {
    try {
      final v = await context.app.vault.run((x) => x.listVersions(page: widget.pageId));
      if (!mounted) return;
      setState(() => _versions = v);
      if (v.isNotEmpty) await _select(0);
    } catch (e) {
      if (mounted) setState(() => _error = 'Could not load the history.');
    }
  }

  Future<void> _select(int i) async {
    final v = _versions;
    if (v == null || i >= v.length) return;
    final g = ++_gen;
    setState(() => _selected = i);
    try {
      final p = await context.app.vault.run((x) => x.loadPageAt(page: widget.pageId, version: v[i].id));
      if (!mounted || g != _gen) return;
      setState(() => _preview = p);
    } catch (e) {
      if (mounted && g == _gen) setState(() => _preview = null);
    }
  }

  Future<void> _restore() async {
    final v = _versions;
    if (v == null || v.isEmpty || _restoring) return;
    setState(() => _restoring = true);
    try {
      await context.app.vault.run((x) => x.restoreVersion(page: widget.pageId, version: v[_selected].id), writes: true);
      if (mounted) Navigator.of(context).pop();
    } catch (e) {
      if (mounted) {
        setState(() {
          _restoring = false;
          _error = 'Could not restore this version.';
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final m = context.mobile;
    final list = _list(context);
    final preview = _previewView(context);
    if (m) {
      return ScreenScaffold(
        title: 'Version history',
        actions: [_restoreButton(context)],
        body: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          SizedBox(height: 200, child: list),
          HLine(color: c.border),
          Expanded(child: preview),
        ]),
      );
    }
    return Row(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
      Container(
        width: 280,
        decoration: BoxDecoration(color: c.bgSecondary, border: Border(right: BorderSide(color: c.borderSubtle))),
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 16, 16, 8),
            child: Text('Version history', style: tsDisplay(const TypeSpec(19, 24, 640), c.textPrimary)),
          ),
          Expanded(child: list),
        ]),
      ),
      Expanded(
        child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [
          Expanded(child: preview),
          Container(
            height: 56,
            padding: const EdgeInsets.symmetric(horizontal: 16),
            decoration: BoxDecoration(border: Border(top: BorderSide(color: c.borderSubtle))),
            child: Row(children: [
              Expanded(
                child: Text(_error ?? _caption(),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: ts(context.type.uiSmall, _error != null ? c.inlineCodeText : c.textTertiary, weight: 400)),
              ),
              DialogButton(key: const ValueKey('history-cancel'), label: 'Cancel', onTap: () => Navigator.of(context).pop()),
              const SizedBox(width: 8),
              _restoreButton(context),
            ]),
          ),
        ]),
      ),
    ]);
  }

  String _caption() {
    final v = _versions;
    if (v == null || v.isEmpty) return '';
    return _selected == 0 ? 'This is the current version.' : 'Restoring keeps the history: it is recorded as a new change.';
  }

  Widget _restoreButton(BuildContext context) {
    final v = _versions;
    final can = v != null && v.isNotEmpty && !_restoring;
    return DialogButton(
      key: const ValueKey('history-restore'),
      label: 'Restore version',
      filled: true,
      height: context.mobile ? 32 : null,
      onTap: can ? _restore : null,
    );
  }

  Widget _list(BuildContext context) {
    final c = context.colors;
    final ty = context.type;
    final v = _versions;
    if (v == null) return const SizedBox.shrink();
    if (v.isEmpty) {
      return Center(child: Text(_error ?? 'No versions yet', style: ts(ty.ui, c.textTertiary, weight: 400)));
    }
    final rows = <Widget>[];
    String? day;
    for (var i = 0; i < v.length; i++) {
      final t = versionTimeOf(v[i].timeMs.toInt());
      final d = _dayLabel(t);
      if (d != day) {
        day = d;
        rows.add(Padding(
          padding: EdgeInsets.fromLTRB(context.mobile ? Layout.gutter : 16, 10, 16, 4),
          child: Text(d, style: ts(ty.uiSmall, c.textTertiary)),
        ));
      }
      rows.add(Padding(
        padding: EdgeInsets.symmetric(horizontal: context.mobile ? 0 : 8, vertical: 1),
        child: PanelRow(
          key: ValueKey('history-version-$i'),
          selected: i == _selected,
          onTap: () => _select(i),
          child: SizedBox(
            height: 48,
            child: Column(mainAxisAlignment: MainAxisAlignment.center, crossAxisAlignment: CrossAxisAlignment.start, children: [
              Text(i == 0 ? '${_time(t)}  ·  Current' : _time(t),
                  style: ts(ty.ui, c.textPrimary, weight: i == _selected ? 600 : 500)),
              Text('${v[i].summary} · ${_device(v[i].device)}',
                  maxLines: 1, overflow: TextOverflow.ellipsis, style: ts(ty.uiSmall, c.textTertiary, weight: 400)),
            ]),
          ),
        ),
      ));
    }
    return ListView(padding: const EdgeInsets.only(bottom: 8), children: rows);
  }

  String _device(String d) {
    final mine = context.app.vault.deviceId;
    if (d == mine) return 'This device';
    return 'Another device';
  }

  static const _months = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

  String _dayLabel(DateTime t) {
    final now = clockNow();
    final today = DateTime(now.year, now.month, now.day);
    final d = DateTime(t.year, t.month, t.day);
    if (d == today) return 'Today';
    if (today.difference(d).inDays == 1) return 'Yesterday';
    return '${_months[t.month - 1]} ${t.day}${t.year == now.year ? '' : ', ${t.year}'}';
  }

  String _time(DateTime t) {
    final h = t.hour % 12 == 0 ? 12 : t.hour % 12;
    return '$h:${t.minute.toString().padLeft(2, '0')} ${t.hour < 12 ? 'AM' : 'PM'}';
  }

  Widget _previewView(BuildContext context) {
    final p = _preview;
    final v = _versions;
    if (p == null || v == null || _selected >= v.length) return const SizedBox.shrink();
    // The real editor, read-only, over the version's blocks.
    return KeyedSubtree(
      key: const ValueKey('history-preview'),
      child: PageScreen(
        key: ValueKey('history-snapshot-${v[_selected].id}'),
        pageId: widget.pageId,
        initialTitle: p.title,
        snapshot: p,
      ),
    );
  }
}
