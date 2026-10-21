import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter/services.dart';

import '../core/change_feed.dart';
import '../core/debounce.dart';
import '../core/models.dart';
import '../editor/page_screen.dart';
import '../state/agenda_store.dart';
import '../state/app_state.dart';
import '../state/page_store.dart';
import '../shell/navigation.dart';
import '../ui/toast.dart';
import '../ui/ui.dart';

/// The selected day's ordinary Journal document, edited with PageScreen.
class TodayDayNote extends StatelessWidget {
  const TodayDayNote({super.key});

  @override
  Widget build(BuildContext context) {
    final day = context.watch<AgendaStore>().selectedDay;
    return _DayNote(key: ValueKey('today-day-note-${dayStamp(day)}'), day: day);
  }
}

class _DayNote extends StatefulWidget {
  const _DayNote({super.key, required this.day});
  final DateTime day;

  @override
  State<_DayNote> createState() => _DayNoteState();
}

class _DayNoteState extends State<_DayNote> {
  final _draft = TextEditingController();
  final _focus = FocusNode();
  final _debounce = Debounce();
  AppState? _app;
  String? _page;
  String? _firstBlock;
  String? _bootstrapPage;
  String? _bootstrapBlock;
  TextEditingValue? _initialEditingValue;
  TextEditingValue _draftValue = TextEditingValue.empty;
  bool _loading = true;
  bool _creating = false;
  bool _opening = false;
  bool _failed = false;
  int _lookupGeneration = 0;

  @override
  void initState() {
    super.initState();
    _draft.addListener(_rememberDraft);
  }

  void _rememberDraft() => _draftValue = _draft.value;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_app != null) return;
    _app = context.app;
    _app!.feed.addListener(_changed);
    unawaited(_lookup());
  }

  void _changed(ChangeBatch batch) {
    if (_creating || _failed || _app!.vault.isClosed) return;
    if (_page == null || batch.touches(_page!) || batch.treeChanged) {
      _debounce.run(
        const Duration(milliseconds: 100),
        () => unawaited(_lookup()),
      );
    }
  }

  Future<void> _lookup() async {
    if (!mounted || _creating || _app!.vault.isClosed) return;
    final generation = ++_lookupGeneration;
    try {
      final id = await _app!.vault.run(
        (v) => v.dailyNote(
          dayMs: dayStamp(widget.day),
          title: dayTitle(widget.day, year: true),
          create: false,
        ),
      );
      if (!mounted || _creating || generation != _lookupGeneration) return;
      setState(() {
        _loading = false;
        _failed = false;
        if (_page != id) {
          _page = id;
          _firstBlock = null;
          _initialEditingValue = null;
        }
      });
    } catch (_) {
      if (!mounted || generation != _lookupGeneration) return;
      setState(() {
        _loading = false;
        _failed = true;
      });
    }
  }

  /// This input exists only before a document does. Keep accepting typing
  /// while creation runs, then hand its exact value to the real block editor.
  Future<void> _create() async {
    if (_creating || (_draftValue.text.isEmpty && _bootstrapPage == null)) {
      return;
    }
    final app = _app!;
    final day = widget.day; // A later selected date cannot redirect this write.
    _creating = true;
    _lookupGeneration++;
    setState(() => _failed = false);
    try {
      final id = (await app.vault.run(
        (v) => v.dailyNote(
          dayMs: dayStamp(day),
          title: dayTitle(day, year: true),
          create: true,
        ),
        writes: true,
      ))!;
      if (_bootstrapPage != id) _bootstrapBlock = null;
      _bootstrapPage = id;
      final data = await app.vault.loadPage(id);
      final roots = data.blocks.where((b) => b.depth == 0).toList();
      final log = roots.indexWhere(
        (b) => b.kind == Kinds.toggle && b.text == 'Log',
      );
      final after = log < 0
          ? (roots.isEmpty ? null : roots.last.id)
          : (log == 0 ? null : roots[log - 1].id);
      // An insert may already have succeeded before a later buffered update
      // failed. Re-read its identity so retry updates that paragraph once.
      final priorBlock = _bootstrapBlock;
      final exists =
          priorBlock != null && data.blocks.any((b) => b.id == priorBlock);
      final block = exists ? priorBlock : newId();
      _bootstrapBlock = block;
      var saved = _draftValue.text;
      await app.vault.apply(id, [
        BOp(
          op: exists ? OpKind.setText : OpKind.insert,
          id: block,
          after: exists ? null : after,
          kind: Kinds.paragraph,
          text: saved,
          marks: const [],
          checked: false,
        ),
      ]);
      while (saved != _draftValue.text) {
        saved = _draftValue.text;
        await app.vault.apply(id, [
          BOp(
            op: OpKind.setText,
            id: block,
            kind: Kinds.paragraph,
            text: saved,
            marks: const [],
            checked: false,
          ),
        ]);
      }
      // The value remains available after disposal, so a rapid day switch
      // still saves the old day's draft without focusing the newly viewed day.
      if (!mounted) return;
      setState(() {
        _page = id;
        _firstBlock = block;
        _initialEditingValue = _draftValue;
        _loading = false;
      });
    } catch (_) {
      if (mounted) {
        setState(() => _failed = true);
        showErrorToast(context, 'Could not save this day’s note. Try again.');
      }
    } finally {
      _creating = false;
    }
  }

  Future<void> _openPage() async {
    final id = _page;
    if (id == null || _opening) return;
    final pages = context.read<PageStore>();
    // Release the inline editor before opening the same document as a page.
    // PageSession's live-document callbacks belong to one editor at a time.
    setState(() => _opening = true);
    await WidgetsBinding.instance.endOfFrame;
    if (!mounted) return;
    await pages.lastSave;
    if (!mounted) return;
    try {
      await openPage(context, id, dayTitle(widget.day, year: true));
    } finally {
      // The full-page editor hands its queued edits back on pop. Finish that
      // save before mounting a new inline session for the same document.
      await pages.lastSave;
      if (mounted) {
        setState(() {
          _opening = false;
          _firstBlock = null;
          _initialEditingValue = null;
        });
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    final c = context.colors;
    final page = _page;
    final stack = PageStackScope.of(context);
    // Other entry points (capture, search, reminders) can also open this page.
    final openAsPage = stack.topId != null;
    if (openAsPage) {
      _firstBlock = null;
      _initialEditingValue = null;
    }
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Row(
          children: [
            Expanded(
              child: Text(
                'Note · ${weekdays[widget.day.weekday - 1].substring(0, 3)} '
                '${widget.day.day} ${months[widget.day.month - 1].substring(0, 3)}',
                style: tsRaw(13, 20, 500, c.textSecondary),
              ),
            ),
            if (page != null)
              TextBtn(
                'Open as page',
                key: const ValueKey('today-note-open-page'),
                color: c.textSecondary,
                onTap: _opening ? null : _openPage,
              ),
          ],
        ),
        const SizedBox(height: 10),
        if (_loading)
          Text('Loading…', style: ts(context.type.uiSmall, c.textTertiary))
        else if (page != null)
          if (!_opening && !openAsPage)
            PageScreen(
              key: ValueKey('today-note-editor-$page'),
              pageId: page,
              initialTitle: dayTitle(widget.day, year: true),
              embedded: true,
              initialBlock: _firstBlock,
              initialEditingValue: _initialEditingValue,
            )
          else
            const SizedBox.shrink()
        else
          Padding(
            padding: EdgeInsets.symmetric(
              horizontal: context.mobile ? 18 : 30,
              vertical: 10,
            ),
            child: PlainField(
              key: const ValueKey('today-note-start'),
              controller: _draft,
              focusNode: _focus,
              style: ts(context.type.body, c.textPrimary),
              placeholder: 'Start writing…',
              textInputAction: TextInputAction.newline,
              onChanged: (_) => unawaited(_create()),
            ),
          ),
        if (_failed)
          Align(
            alignment: Alignment.centerLeft,
            child: TextBtn(
              'Retry',
              key: const ValueKey('today-note-retry'),
              onTap: () => _bootstrapPage != null || _draftValue.text.isNotEmpty
                  ? _create()
                  : _lookup(),
            ),
          ),
      ],
    );
  }

  @override
  void dispose() {
    _lookupGeneration++;
    _debounce.cancel();
    _app?.feed.removeListener(_changed);
    _draft.removeListener(_rememberDraft);
    _draft.dispose();
    _focus.dispose();
    super.dispose();
  }
}
