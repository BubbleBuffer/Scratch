// Windows updates: checks GitHub for a newer Scratch release, downloads
// and verifies its installer, and starts it when the user clicks "Restart
// to update". The work is in Rust (app/rust/src/updater.rs); this decides
// when to call it and holds the state the settings section and the
// sidebar indicator show. How and why: wiki/releases/windows-install.md.
//
// - Release and nightly builds are built with
//   `--dart-define=SCRATCH_RELEASE_TAG=<tag>`. Without it the build is a
//   development build and never checks, unless SCRATCH_UPDATE_TAG is set in
//   the environment (a debug override that pretends to be that tag).
// - Checks run 30 s after start, then every 6 hours, and on "Check now".
//   Offline and rate-limited checks are retried at the next one, quietly.
// - Downloads happen in the background when "Download updates
//   automatically" is on, else on the user's click. Installing is always
//   the user's click.

import 'dart:async';
import 'dart:io';
import 'dart:ui' show AppExitType;

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../../src/rust/api/update.dart' as rust;
import '../settings/settings_store.dart';

/// The release tag this build was published as ('' for development builds).
const buildReleaseTag = String.fromEnvironment('SCRATCH_RELEASE_TAG');

/// [buildReleaseTag], or the SCRATCH_UPDATE_TAG debug override.
String effectiveReleaseTag() {
  final forced = Platform.environment['SCRATCH_UPDATE_TAG'];
  if (forced != null && forced.isNotEmpty) return forced;
  return buildReleaseTag;
}

bool isNightlyTag(String tag) => tag.contains('-nightly.');

/// The calls into Rust, replaceable in tests.
abstract class UpdateBackend {
  Future<rust.UpdateCheck> check({required String currentTag, required bool includeNightly, required String updatesDir});
  Future<rust.DownloadedUpdate> download(rust.UpdateRelease release, {required String currentTag, required String updatesDir});
  rust.UpdateProgress progress();
  Future<void> install({required String path, required String sha256});

  /// Closes the app the normal way (the exit listener flushes the vault).
  Future<void> exitApp();
}

class RustUpdateBackend implements UpdateBackend {
  const RustUpdateBackend();

  @override
  Future<rust.UpdateCheck> check({required String currentTag, required bool includeNightly, required String updatesDir}) =>
      rust.updateCheck(currentTag: currentTag, includeNightly: includeNightly, updatesDir: updatesDir);

  @override
  Future<rust.DownloadedUpdate> download(rust.UpdateRelease release, {required String currentTag, required String updatesDir}) =>
      rust.updateDownload(release: release, currentTag: currentTag, updatesDir: updatesDir);

  @override
  rust.UpdateProgress progress() => rust.updateDownloadProgress();

  @override
  Future<void> install({required String path, required String sha256}) => rust.updateInstall(path: path, sha256: sha256);

  @override
  Future<void> exitApp() async {
    await ServicesBinding.instance.exitApplication(AppExitType.cancelable);
  }
}

enum UpdatePhase { development, idle, checking, upToDate, available, downloading, ready, installing }

class UpdateController extends ChangeNotifier {
  UpdateController({
    required this.currentTag,
    required this.updatesDir,
    required this.settings,
    this.backend = const RustUpdateBackend(),
    DateTime Function()? now,
  }) : _now = now ?? DateTime.now,
       phase = currentTag.isEmpty ? UpdatePhase.development : UpdatePhase.idle;

  /// The app's controller (Windows only; null elsewhere and in most tests).
  static UpdateController? current;

  static const firstCheckDelay = Duration(seconds: 30);
  static const checkInterval = Duration(hours: 6);

  final String currentTag;
  final String updatesDir;
  final AppSettings settings;
  final UpdateBackend backend;
  final DateTime Function() _now;

  /// The controller's clock (injectable for tests).
  DateTime now() => _now();

  UpdatePhase phase;
  DateTime? checkedAt;

  /// The release on offer (available, downloading or ready).
  rust.UpdateRelease? release;

  /// The verified installer, once downloaded.
  rust.DownloadedUpdate? downloaded;

  /// Download progress, 0..1 (null when the size is unknown).
  double? progress;

  /// The last problem (offline, rate limit, a failed download), shown in
  /// settings only. Cleared by the next success.
  String? problem;

  Timer? _first, _every, _poll;
  bool _disposed = false;

  bool get enabled => currentTag.isNotEmpty;
  bool get ready => phase == UpdatePhase.ready;
  bool get busy => phase == UpdatePhase.checking || phase == UpdatePhase.downloading || phase == UpdatePhase.installing;

  /// Starts the schedule: the first check after [firstCheckDelay], then
  /// one every [checkInterval].
  void start() {
    if (!enabled) return;
    _first = Timer(firstCheckDelay, check);
    _every = Timer.periodic(checkInterval, (_) => check());
  }

  void _changed() {
    if (!_disposed) notifyListeners();
  }

  /// Checks GitHub now. Does nothing while busy.
  Future<void> check() async {
    if (!enabled || busy) return;
    final before = phase;
    phase = UpdatePhase.checking;
    _changed();
    rust.UpdateCheck r;
    try {
      r = await backend.check(currentTag: currentTag, includeNightly: settings.updatesIncludeNightly, updatesDir: updatesDir);
    } catch (e) {
      r = rust.UpdateCheck(status: rust.UpdateCheckStatus.failed, message: '$e');
    }
    if (_disposed) return;
    switch (r.status) {
      case rust.UpdateCheckStatus.upToDate:
        checkedAt = _now();
        release = null;
        downloaded = null;
        problem = null;
        phase = UpdatePhase.upToDate;
      case rust.UpdateCheckStatus.available:
        checkedAt = _now();
        problem = null;
        final offered = r.release!;
        final same = downloaded != null && release?.tag == offered.tag;
        release = offered;
        if (same) {
          phase = UpdatePhase.ready;
        } else {
          downloaded = null;
          phase = UpdatePhase.available;
          if (settings.updatesAutoDownload) {
            _changed();
            unawaited(download());
            return;
          }
        }
      case rust.UpdateCheckStatus.offline:
      case rust.UpdateCheckStatus.rateLimited:
      case rust.UpdateCheckStatus.failed:
        // Quiet: keep what we knew, note the problem for settings.
        problem = r.message ?? 'The check failed';
        phase = before == UpdatePhase.checking ? UpdatePhase.idle : before;
      case rust.UpdateCheckStatus.unsupported:
        problem = r.message;
        phase = UpdatePhase.development;
    }
    _changed();
  }

  /// Downloads and verifies the offered installer.
  Future<void> download() async {
    final r = release;
    if (r == null || phase == UpdatePhase.downloading || phase == UpdatePhase.installing) return;
    phase = UpdatePhase.downloading;
    progress = 0;
    _changed();
    _poll = Timer.periodic(const Duration(milliseconds: 250), (_) {
      final p = backend.progress();
      final total = p.total.toInt();
      progress = total > 0 ? (p.received.toInt() / total).clamp(0.0, 1.0) : null;
      _changed();
    });
    try {
      downloaded = await backend.download(r, currentTag: currentTag, updatesDir: updatesDir);
      problem = null;
      phase = UpdatePhase.ready;
    } catch (e) {
      downloaded = null;
      problem = '$e';
      phase = UpdatePhase.available;
    } finally {
      _poll?.cancel();
      _poll = null;
      progress = null;
    }
    _changed();
  }

  /// Starts the installer and closes the app. Only from the user's click.
  Future<void> install() async {
    final d = downloaded;
    if (d == null || phase != UpdatePhase.ready) return;
    phase = UpdatePhase.installing;
    _changed();
    try {
      await backend.install(path: d.path, sha256: d.sha256);
    } catch (e) {
      // The installer was checked again before starting; a mismatch means
      // the file changed on disk, so it must be downloaded again.
      problem = '$e';
      downloaded = null;
      phase = UpdatePhase.available;
      _changed();
      return;
    }
    await backend.exitApp();
  }

  @override
  void dispose() {
    _disposed = true;
    _first?.cancel();
    _every?.cancel();
    _poll?.cancel();
    super.dispose();
  }
}
