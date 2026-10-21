import 'navigation.dart';

// Dev replay orchestration: every input uses the ordinary send/run/event path.
import 'dart:async';
import 'dart:io';

import 'package:flutter/widgets.dart';

import '../../core/file_dialogs.dart';
import '../../state/app_state.dart';
import '../../src/rust/api/agent.dart';
import '../../ui/ui.dart';
import '../../ui/toast.dart';
import 'agent_store.dart';

/// One runtime flag gates both the command and the native provider registry.
bool get devReplayEnabled => Platform.environment['NOTES_DEV_REPLAY'] == '1';

class ReplayPlayback extends ChangeNotifier {
  ReplayPlayback(
    this.store,
    this.agent,
    this.sessionId,
    this.userTexts,
    this.delay,
  );
  final AgentStore store;
  final NotesAgent agent;
  final String sessionId;
  final List<String> userTexts;
  final Duration delay;
  int sent = 0;
  bool paused = false;
  bool stopped = false;
  bool finished = false;
  String? error;
  bool _controlBusy = false;
  Future<void>? _done;
  Completer<void>? _waiting;
  Future<void> get done => _done ?? Future.value();
  bool get active => !stopped && !finished && error == null;

  void start() => _done ??= _drive();

  Future<void> _drive() async {
    try {
      for (final text in userTexts) {
        await _waitWhilePaused();
        if (stopped) break;
        await store.send(text, sessionId: sessionId);
        if (store.error != null) throw StateError(store.error!);
        while (!stopped) {
          await agent.replayStatus(sessionId: sessionId);
          final state = await agent.sessionState(sessionId: sessionId);
          final running = state.runs.any(runActive);
          if (!running) {
            final last = state.runs.lastOrNull;
            if (last?.status != 'completed') {
              throw StateError(last?.error ?? 'The replay run did not finish.');
            }
            break;
          }
          await _sleep(const Duration(milliseconds: 60));
        }
        if (stopped) break;
        sent++;
        notifyListeners();
        // Pause also suspends the inter-message gap; stop never waits for it.
        var remaining = delay;
        while (remaining > Duration.zero && !stopped) {
          await _waitWhilePaused();
          if (stopped) break;
          final slice = remaining > const Duration(milliseconds: 60)
              ? const Duration(milliseconds: 60)
              : remaining;
          await _sleep(slice);
          remaining -= slice;
        }
      }
      if (!stopped) {
        await agent.replayStatus(sessionId: sessionId);
        finished = true;
      }
    } catch (e) {
      if (!stopped) {
        error = '$e';
        try {
          await agent.replayControl(sessionId: sessionId, action: 'stop');
        } catch (_) {}
        try {
          await agent.cancel(sessionId: sessionId);
        } catch (_) {}
      }
    }
    notifyListeners();
  }

  Future<void> _waitWhilePaused() async {
    while (paused && !stopped) {
      await _sleep(const Duration(milliseconds: 60));
    }
  }

  Future<void> _sleep(Duration duration) async {
    if (stopped) return;
    final waiting = Completer<void>();
    _waiting = waiting;
    final timer = Timer(duration, () {
      if (!waiting.isCompleted) waiting.complete();
    });
    try {
      await waiting.future;
    } finally {
      timer.cancel();
      if (identical(_waiting, waiting)) _waiting = null;
    }
  }

  Future<void> togglePause() async {
    if (!active || _controlBusy) return;
    _controlBusy = true;
    final next = !paused;
    // Gate the next send immediately, including the native-control round trip.
    paused = next;
    notifyListeners();
    try {
      await agent.replayControl(
        sessionId: sessionId,
        action: next ? 'pause' : 'resume',
      );
    } catch (e) {
      error = '$e';
      await stop();
    } finally {
      _controlBusy = false;
    }
    notifyListeners();
  }

  Future<void> stop() async {
    if (stopped || finished) return;
    stopped = true;
    paused = false;
    final waiting = _waiting;
    if (waiting != null && !waiting.isCompleted) waiting.complete();
    notifyListeners();
    try {
      await agent.replayControl(sessionId: sessionId, action: 'stop');
    } catch (e) {
      error ??= '$e';
    }
    // A send may still be starting when Stop is tapped. Wait for the driver
    // to leave that send before cancelling, so a late-created run is caught.
    await done;
    try {
      await agent.cancel(sessionId: sessionId);
      final deadline = DateTime.now().add(const Duration(seconds: 10));
      while ((await agent.sessionState(sessionId: sessionId)).runs
          .any(runActive)) {
        if (DateTime.now().isAfter(deadline)) {
          throw StateError('The replay run has not stopped yet.');
        }
        await Future<void>.delayed(const Duration(milliseconds: 60));
      }
    } catch (e) {
      error ??= '$e';
    }
  }
}

Future<void> showReplayCommand(BuildContext context) async {
  if (!devReplayEnabled) return;
  final store = AgentStore.of(context.app);
  final files = await FileDialogs.instance.pickFiles(
    title: 'Replay chat live',
    extensions: const ['sqlite'],
  );
  if (!context.mounted || files.isEmpty) return;
  final path = files.first.path;
  if (path == null) return;
  final delay = await showPickerSheet<int>(
    context,
    (_) => _ReplaySpeedPicker(name: files.first.name),
    desktopWidth: 440,
  );
  if (delay == null || !context.mounted) return;
  try {
    await store.startReplay(path, delayMs: delay);
    if (context.mounted) unawaited(openAgent(context));
  } catch (e) {
    if (context.mounted) showErrorToast(context, 'Replay: $e');
  }
}

class _ReplaySpeedPicker extends StatefulWidget {
  const _ReplaySpeedPicker({required this.name});
  final String name;
  @override
  State<_ReplaySpeedPicker> createState() => _ReplaySpeedPickerState();
}

class _ReplaySpeedPickerState extends State<_ReplaySpeedPicker> {
  final _controller = TextEditingController(text: '300');
  final _focus = FocusNode();
  String? _error;
  @override
  void dispose() {
    _controller.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _start() {
    final ms = int.tryParse(_controller.text);
    if (ms == null || ms < 1 || ms > 60000) {
      setState(() => _error = 'Enter a delay from 1 to 60,000 ms.');
      return;
    }
    Navigator.of(context).pop(ms);
  }

  @override
  Widget build(BuildContext context) => PickerSheet(
    header: const SheetHeader('Replay chat live'),
    list: Padding(
      padding: const EdgeInsets.symmetric(horizontal: 12),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text(
            widget.name,
            style: ts(context.type.ui, context.colors.textPrimary),
          ),
          const SizedBox(height: 12),
          Text(
            'Delay in milliseconds',
            style: ts(context.type.ui, context.colors.textSecondary),
          ),
          const SizedBox(height: 6),
          InputBox(
            child: PlainField(
              key: const ValueKey('replay-delay'),
              controller: _controller,
              focusNode: _focus,
              style: ts(context.type.ui, context.colors.textPrimary),
              placeholder: '300',
              onSubmitted: (_) => _start(),
            ),
          ),
          const SizedBox(height: 8),
          Text(
            _error ?? 'Each paragraph and the gap between user messages use this delay.',
            style: ts(context.type.ui, context.colors.textSecondary),
          ),
        ],
      ),
    ),
    footer: PrimaryButton(label: 'Start replay', onTap: _start),
  );
}

class ReplayControls extends StatelessWidget {
  const ReplayControls({super.key, required this.playback});
  final ReplayPlayback playback;
  @override
  Widget build(BuildContext context) => Padding(
    key: const ValueKey('replay-controls'),
    padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 6),
    child: Row(
      children: [
        Expanded(
          child: Text(
            playback.error ??
                (playback.finished
                    ? 'Replay finished'
                    : playback.stopped
                    ? 'Replay stopped'
                    : 'Replay ${playback.sent}/${playback.userTexts.length}${playback.paused ? ' · paused' : ''}'),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
            style: ts(context.type.ui, context.colors.textSecondary),
          ),
        ),
        if (playback.active) ...[
          TextBtn(
            playback.paused ? 'Resume' : 'Pause',
            onTap: () => unawaited(playback.togglePause()),
          ),
          TextBtn('Stop', onTap: () => unawaited(playback.stop())),
        ],
      ],
    ),
  );
}
