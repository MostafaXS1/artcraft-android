/*
 * Coiman IDE's Termux shim.
 *
 * The Android app cannot execute the programs Coiman needs (git, cargo, python, …): they live in
 * Termux, a different app. This shim is bundled as a native library, executes as the Coiman app,
 * and makes one Termux invocation look exactly like a local process:
 *
 *   - it creates a scratch directory in the shared folder both apps can see
 *     (/sdcard/Documents/CoimanTermux) with a request directory in the app's own files folder;
 *   - the Activity's TermuxBroker sees the request and starts Termux's official RUN_COMMAND
 *     service with the generated `run.sh`;
 *   - this process copies its own stdin into the shared `in` file and tails the shared `out` and
 *     `err` files, so a parent using Command::output()/spawn() sees a normal, streaming child
 *     process;
 *   - when the remote command ends, the shim exits with its status.
 *
 * Shared storage is the only place both apps can write, and Android's shared storage has no named
 * pipes (mkfifo is refused). The stdin bridge is therefore a regular file: the shim appends the
 * bytes it receives to <scratch>/in, and the generated script feeds that file into a FIFO it
 * creates inside Termux's own home directory (a normal filesystem, where pipes work). The script
 * ends the feed when the shim writes <scratch>/in.done, so the remote command sees end-of-file on
 * stdin exactly as it would locally.
 *
 * The file is compiled twice by prepare.sh: libcoiman_termux_shim.so (tools) and
 * libcoiman_termux_shell.so (interactive terminal). The same binary serves both; the tool name is
 * in COIMAN_TERMUX_TOOL and "__terminal__" selects the `script`-wrapped interactive shell.
 *
 * ARTCRAFT_COIMANIDE_TERMUX_SHIM_V2
 */
#define _GNU_SOURCE
#include <dirent.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <signal.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#define PATH_MAX_LEN 1024
#define TOOL_MAX 256
#define TERMUX_BASH "/data/data/com.termux/files/usr/bin/bash"
#define TERMUX_HOME "/data/data/com.termux/files/home"
#define DEFAULT_PUBLIC "/sdcard/Documents/CoimanTermux"

static char g_kill_path[PATH_MAX_LEN];
static char g_alive_path[PATH_MAX_LEN];
static volatile sig_atomic_t g_stop = 0;
static volatile sig_atomic_t g_remote_done = 0;

static void fail(const char *format, ...) {
    va_list args;
    va_start(args, format);
    fprintf(stderr, "Coiman IDE: ");
    vfprintf(stderr, format, args);
    fprintf(stderr, "\n");
    va_end(args);
}

static long long now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (long long)ts.tv_sec * 1000LL + ts.tv_nsec / 1000000LL;
}

static void sleep_ms(long ms) {
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    nanosleep(&ts, NULL);
}

static int mkdirs(const char *path) {
    char buffer[PATH_MAX_LEN];
    if (strlen(path) >= sizeof(buffer)) return -1;
    snprintf(buffer, sizeof(buffer), "%s", path);
    for (char *p = buffer + 1; *p; p++) {
        if (*p != '/') continue;
        *p = '\0';
        if (mkdir(buffer, 0777) != 0 && errno != EEXIST) return -1;
        *p = '/';
    }
    if (mkdir(buffer, 0777) != 0 && errno != EEXIST) return -1;
    return 0;
}

static int write_file(const char *path, const char *data, size_t length) {
    int fd = open(path, O_WRONLY | O_CREAT | O_TRUNC, 0666);
    if (fd < 0) return -1;
    size_t written = 0;
    while (written < length) {
        ssize_t n = write(fd, data + written, length - written);
        if (n < 0) {
            if (errno == EINTR) continue;
            close(fd);
            return -1;
        }
        written += (size_t)n;
    }
    close(fd);
    return 0;
}

static int read_file(const char *path, char *buffer, size_t capacity) {
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;
    ssize_t n;
    size_t used = 0;
    while (used + 1 < capacity && (n = read(fd, buffer + used, capacity - used - 1)) > 0) used += (size_t)n;
    close(fd);
    buffer[used] = '\0';
    return (int)used;
}

static bool file_exists(const char *path) {
    return access(path, F_OK) == 0;
}

static void touch_file(const char *path) {
    int fd = open(path, O_WRONLY | O_CREAT, 0666);
    if (fd >= 0) close(fd);
}

/* Removes one scratch directory (files first, then the directory). The shim owns it, and the
 * broker sweeps anything a killed process left behind. */
static void remove_tree(const char *path) {
    DIR *dir = opendir(path);
    if (!dir) {
        unlink(path);
        return;
    }
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        char child[PATH_MAX_LEN];
        snprintf(child, sizeof(child), "%s/%s", path, entry->d_name);
        struct stat info;
        if (lstat(child, &info) == 0 && S_ISDIR(info.st_mode))
            remove_tree(child);
        else
            unlink(child);
    }
    closedir(dir);
    rmdir(path);
}

static void shell_quote(FILE *file, const char *text) {
    fputc('\'', file);
    for (const char *p = text; *p; p++) {
        if (*p == '\'')
            fputs("'\\''", file);
        else
            fputc(*p, file);
    }
    fputc('\'', file);
}

static bool env_forwarded(const char *name) {
    /* The locale is deliberately not forwarded: Termux's own locale files live in packages the
     * user may not have installed, and a shell that is told about a locale it does not know
     * prints "setlocale: LC_ALL: cannot change locale (en-GB)" on every start. */
    static const char *prefixes[] = {
        "GIT_", "GH_", "CARGO_", "RUST", "PYTHON", "JAVA_", "MIX_", "NODE_", "NPM_",
        "GO", "CC", "CXX", "CFLAGS", "LDFLAGS", "PKG_CONFIG", "TERM", "COLORTERM", "NO_COLOR",
        "CLICOLOR", "FORCE_COLOR", "CI", "PAGER", "EDITOR",
    };
    for (size_t i = 0; i < sizeof(prefixes) / sizeof(prefixes[0]); i++) {
        if (strncmp(name, prefixes[i], strlen(prefixes[i])) == 0) return true;
    }
    return false;
}

static void write_export(FILE *file, const char *entry) {
    const char *equals = strchr(entry, '=');
    if (!equals || equals == entry) return;
    size_t name_length = (size_t)(equals - entry);
    if (name_length >= 128) return;
    char name[128];
    memcpy(name, entry, name_length);
    name[name_length] = '\0';
    for (size_t i = 0; i < name_length; i++) {
        char c = name[i];
        if (!((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '_'))
            return;
    }
    fputs("export ", file);
    fputs(name, file);
    fputc('=', file);
    shell_quote(file, equals + 1);
    fputc('\n', file);
}

static void on_signal(int signal_number) {
    (void)signal_number;
    g_stop = 1;
    if (g_kill_path[0]) {
        int fd = open(g_kill_path, O_WRONLY | O_CREAT, 0666);
        if (fd >= 0) {
            ssize_t ignored = write(fd, "1", 1);
            (void)ignored;
            close(fd);
        }
    }
}

struct tail_args {
    const char *path;
    int output;
    const char *exit_path;
    const char *result_exit_path;
};

static void *tail_file(void *raw) {
    struct tail_args *args = (struct tail_args *)raw;
    int fd = -1;
    char buffer[65536];
    for (;;) {
        if (fd < 0) {
            fd = open(args->path, O_RDONLY);
            if (fd < 0) {
                if (file_exists(args->exit_path) || file_exists(args->result_exit_path) || g_stop || g_remote_done)
                    break;
                sleep_ms(10);
                continue;
            }
        }
        ssize_t n = read(fd, buffer, sizeof(buffer));
        if (n > 0) {
            size_t written = 0;
            while (written < (size_t)n) {
                ssize_t w = write(args->output, buffer + written, (size_t)n - written);
                if (w < 0) {
                    if (errno == EINTR) continue;
                    break;
                }
                written += (size_t)w;
            }
            continue;
        }
        if (n < 0 && errno == EINTR) continue;
        if (file_exists(args->exit_path) || file_exists(args->result_exit_path) || g_stop || g_remote_done) {
            /* One final read after the remote wrote its exit marker. */
            n = read(fd, buffer, sizeof(buffer));
            if (n > 0) {
                size_t written = 0;
                while (written < (size_t)n) {
                    ssize_t w = write(args->output, buffer + written, (size_t)n - written);
                    if (w < 0) {
                        if (errno == EINTR) continue;
                        break;
                    }
                    written += (size_t)w;
                }
            }
            break;
        }
        sleep_ms(10);
    }
    if (fd >= 0) close(fd);
    return NULL;
}

struct input_args {
    const char *input_path;
    const char *done_path;
};

/* Copies this process's stdin into the shared file the remote side reads, and marks the end of
 * the stream when there is no more. */
static void *forward_stdin(void *raw) {
    struct input_args *args = (struct input_args *)raw;
    int fd = open(args->input_path, O_WRONLY | O_CREAT | O_APPEND, 0666);
    if (fd < 0) return NULL;
    char buffer[65536];
    for (;;) {
        ssize_t n = read(STDIN_FILENO, buffer, sizeof(buffer));
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) break;
        size_t written = 0;
        while (written < (size_t)n) {
            ssize_t w = write(fd, buffer + written, (size_t)n - written);
            if (w < 0) {
                if (errno == EINTR) continue;
                close(fd);
                return NULL;
            }
            written += (size_t)w;
        }
    }
    close(fd);
    touch_file(args->done_path);
    return NULL;
}

static void *heartbeat(void *raw) {
    (void)raw;
    /* The watchdog on the other side reads this file's *contents* (a timestamp in seconds), not
     * its modification time: shared storage caches file attributes, so an mtime can lag behind
     * by seconds and would look like a stalled session. */
    while (!g_stop && !g_remote_done) {
        char stamp[32];
        snprintf(stamp, sizeof(stamp), "%lld", (long long)time(NULL));
        write_file(g_alive_path, stamp, strlen(stamp));
        for (int i = 0; i < 20 && !g_stop && !g_remote_done; i++) sleep_ms(100);
    }
    touch_file(g_alive_path);
    return NULL;
}

static int read_exit_code(const char *path) {
    char buffer[64];
    if (read_file(path, buffer, sizeof(buffer)) <= 0) return -1;
    char *end = NULL;
    long value = strtol(buffer, &end, 10);
    if (end == buffer) return -1;
    return (int)value;
}

/* Whether Termux can see this path at all: shared storage is the only place both apps can
 * reach. The roots can be overridden (colon separated) for a different mount. */
static bool path_is_shared(const char *path) {
    static const char *default_roots = "/storage:/sdcard";
    const char *roots = getenv("COIMAN_TERMUX_SHARED_ROOTS");
    if (!roots || !*roots) roots = default_roots;
    const char *cursor = roots;
    while (*cursor) {
        const char *end = strchr(cursor, ':');
        size_t length = end ? (size_t)(end - cursor) : strlen(cursor);
        if (length > 0 && strncmp(path, cursor, length) == 0) {
            char next = path[length];
            if (next == '/' || next == '\0') return true;
        }
        if (!end) break;
        cursor = end + 1;
    }
    return false;
}

int main(int argc, char **argv) {
    const char *tool = getenv("COIMAN_TERMUX_TOOL");
    if (!tool || !*tool) tool = "bash";
    const char *public_root = getenv("COIMAN_TERMUX_PUBLIC");
    if (!public_root || !*public_root) public_root = DEFAULT_PUBLIC;
    const char *request_root = getenv("COIMAN_TERMUX_REQ");
    if (!request_root || !*request_root) {
        fail("COIMAN_TERMUX_REQ is not set; this build cannot reach Termux.");
        return 127;
    }

    char status_path[PATH_MAX_LEN];
    snprintf(status_path, sizeof(status_path), "%s/status", public_root);
    char status[2048];
    if (read_file(status_path, status, sizeof(status)) > 0 && strncmp(status, "error:", 6) == 0) {
        char *message = status + 6;
        while (*message == ' ') message++;
        char *newline = strchr(message, '\n');
        if (newline) *newline = '\0';
        fail("%s", message[0] ? message : "Termux cannot run commands for Coiman IDE.");
        return 127;
    }

    char scratch_root[PATH_MAX_LEN];
    snprintf(scratch_root, sizeof(scratch_root), "%s/scratch", public_root);
    if (mkdirs(scratch_root) != 0) {
        fail("cannot create %s: %s", scratch_root, strerror(errno));
        return 127;
    }
    if (mkdirs(request_root) != 0) {
        fail("cannot create %s: %s", request_root, strerror(errno));
        return 127;
    }

    long long stamp = now_ms();
    char token[128];
    snprintf(token, sizeof(token), "%d-%lld-%ld", (int)getpid(), stamp, (long)(stamp % 9973));

    char scratch[PATH_MAX_LEN];
    char request[PATH_MAX_LEN];
    char temp_request[PATH_MAX_LEN];
    snprintf(scratch, sizeof(scratch), "%s/%s", scratch_root, token);
    snprintf(request, sizeof(request), "%s/%s", request_root, token);
    snprintf(temp_request, sizeof(temp_request), "%s/.%s.tmp", request_root, token);
    if (mkdirs(scratch) != 0 || mkdirs(temp_request) != 0) {
        fail("cannot create the Termux request directory: %s", strerror(errno));
        return 127;
    }

    char in_path[PATH_MAX_LEN];
    char in_done_path[PATH_MAX_LEN];
    char out_path[PATH_MAX_LEN];
    char err_path[PATH_MAX_LEN];
    char exit_path[PATH_MAX_LEN];
    char started_path[PATH_MAX_LEN];
    char kill_path[PATH_MAX_LEN];
    char alive_path[PATH_MAX_LEN];
    char script_path[PATH_MAX_LEN];
    char result_dir[PATH_MAX_LEN];
    char result_exit[PATH_MAX_LEN];
    char result_error[PATH_MAX_LEN];
    char broker_error[PATH_MAX_LEN];
    snprintf(in_path, sizeof(in_path), "%s/in", scratch);
    snprintf(in_done_path, sizeof(in_done_path), "%s/in.done", scratch);
    snprintf(out_path, sizeof(out_path), "%s/out", scratch);
    snprintf(err_path, sizeof(err_path), "%s/err", scratch);
    snprintf(exit_path, sizeof(exit_path), "%s/exit", scratch);
    snprintf(started_path, sizeof(started_path), "%s/started", scratch);
    snprintf(kill_path, sizeof(kill_path), "%s/kill", scratch);
    snprintf(alive_path, sizeof(alive_path), "%s/alive", scratch);
    snprintf(script_path, sizeof(script_path), "%s/run.sh", scratch);
    snprintf(result_dir, sizeof(result_dir), "%s/result", scratch);
    snprintf(result_exit, sizeof(result_exit), "%s/exit_code", result_dir);
    snprintf(result_error, sizeof(result_error), "%s/err", result_dir);
    snprintf(broker_error, sizeof(broker_error), "%s/error", scratch);
    snprintf(g_kill_path, sizeof(g_kill_path), "%s", kill_path);
    snprintf(g_alive_path, sizeof(g_alive_path), "%s", alive_path);

    touch_file(in_path);
    touch_file(alive_path);
    mkdirs(result_dir);

    char cwd[PATH_MAX_LEN];
    if (!getcwd(cwd, sizeof(cwd))) snprintf(cwd, sizeof(cwd), "%s", TERMUX_HOME);
    const char *workdir = path_is_shared(cwd) ? cwd : "";
    if (!workdir[0] && (strncmp(cwd, "/data/", 6) == 0 || strstr(cwd, "/Android/data/") != NULL)) {
        /* The app's own storage is invisible to Termux (another app's files). Saying so beats
         * running the command in Termux's home folder without a word. */
        fail(
            "Coiman IDE: Termux cannot reach %s (it is the app's private storage); the command "
            "runs in Termux's home folder instead.",
            cwd);
    }

    int columns = 80, lines = 24;
    const char *size = getenv("COIMAN_TERMUX_SIZE");
    if (size) sscanf(size, "%dx%d", &columns, &lines);
    if (columns < 2 || columns > 1000) columns = 80;
    if (lines < 2 || lines > 1000) lines = 24;

    FILE *script = fopen(script_path, "w");
    if (!script) {
        fail("cannot write %s: %s", script_path, strerror(errno));
        return 127;
    }
    fprintf(script, "#!%s\n", TERMUX_BASH);
    fprintf(script, "# Generated by Coiman IDE for one Termux invocation.\n");
    fprintf(script, "echo 1 > ");
    shell_quote(script, started_path);
    fprintf(script, "\nexport PATH=\"$PREFIX/bin:$HOME/.cargo/bin:$HOME/.local/bin:$PATH\"\n");
    /* A project folder lives in shared storage, so it belongs to the editor app and not to
     * Termux. Without this, git refuses to work in it ("detected dubious ownership"). */
    fprintf(script, "if [ -z \"${GIT_CONFIG_COUNT:-}\" ]; then\n");
    fprintf(script, "  export GIT_CONFIG_COUNT=1\n");
    fprintf(script, "  export GIT_CONFIG_KEY_0=safe.directory\n");
    fprintf(script, "  export GIT_CONFIG_VALUE_0='*'\n");
    fprintf(script, "fi\n");
    for (char **entry = environ; entry && *entry; entry++) {
        if (strncmp(*entry, "COIMAN_TERMUX_", 14) == 0) continue;
        if (env_forwarded(*entry)) write_export(script, *entry);
    }
    if (workdir[0]) {
        fprintf(script, "cd ");
        shell_quote(script, workdir);
        fprintf(script, " 2>/dev/null || cd \"$HOME\"\n");
    } else {
        fprintf(script, "cd \"$HOME\"\n");
    }

    /* The stdin bridge: a FIFO inside Termux's own filesystem (shared storage has no named
     * pipes), fed from the shared file the shim appends to. Termux's temporary directory is
     * preferred: it is writable, private and always there. */
    fprintf(script, "fifo_dir=\"${PREFIX:-/data/data/com.termux/files/usr}/tmp\"\n");
    fprintf(script, "[ -d \"$fifo_dir\" ] || fifo_dir=\"$HOME\"\n");
    fprintf(script, "mkdir -p \"$fifo_dir\" 2>/dev/null\n");
    fprintf(script, "fifo=\"$fifo_dir/coiman-");
    shell_quote(script, token);
    fprintf(script, ".in\"\n");
    fprintf(script, "rm -f \"$fifo\"\n");
    fprintf(script, "if mkfifo \"$fifo\" 2>/dev/null; then\n");
    fprintf(script, "  (\n");
    fprintf(script, "    exec 3> \"$fifo\" || exit 0\n");
    fprintf(script, "    sent=0\n");
    fprintf(script, "    while :; do\n");
    fprintf(script, "      if [ -f ");
    shell_quote(script, in_path);
    fprintf(script, " ]; then\n");
    fprintf(script, "        size=$(wc -c < ");
    shell_quote(script, in_path);
    fprintf(script, " 2>/dev/null || echo 0)\n");
    fprintf(script, "        size=${size//[^0-9]/}\n");
    fprintf(script, "        if [ -n \"$size\" ] && [ \"$size\" -gt \"$sent\" ]; then\n");
    fprintf(script, "          tail -c +$((sent + 1)) ");
    shell_quote(script, in_path);
    fprintf(script, " 2>/dev/null | head -c $((size - sent)) >&3 2>/dev/null\n");
    fprintf(script, "          sent=$size\n");
    fprintf(script, "        fi\n");
    fprintf(script, "      fi\n");
    fprintf(script, "      if [ -f ");
    shell_quote(script, in_done_path);
    fprintf(script, " ]; then break; fi\n");
    fprintf(script, "      sleep 0.2\n");
    fprintf(script, "    done\n");
    fprintf(script, "    exec 3>&-\n");
    fprintf(script, "  ) &\n");
    fprintf(script, "  feeder=$!\n");
    fprintf(script, "  stdin_source=\"$fifo\"\n");
    fprintf(script, "else\n");
    fprintf(script, "  echo ");
    shell_quote(script, "Coiman IDE: Termux could not create its input pipe; the command runs without input.");
    fprintf(script, " > ");
    shell_quote(script, err_path);
    fprintf(script, "\n  feeder=''\n");
    fprintf(script, "  stdin_source=/dev/null\n");
    fprintf(script, "fi\n");
    /* The interactive shell needs a pipe that stays open: with /dev/null it would read end of
     * file at once and exit, and Coiman closes the terminal panel when its shell ends. */
    if (strcmp(tool, "__terminal__") == 0) {
        fprintf(script, "if [ \"$stdin_source\" = /dev/null ]; then\n");
        fprintf(script, "  stdin_source=\"$fifo_dir/coiman-");
        shell_quote(script, token);
        fprintf(script, ".keep\"\n");
        fprintf(script, "  rm -f \"$stdin_source\"\n");
        fprintf(script, "  mkfifo \"$stdin_source\" 2>/dev/null && (sleep 86400 > \"$stdin_source\" &) || stdin_source=/dev/null\n");
        fprintf(script, "fi\n");
    }

    if (strcmp(tool, "__terminal__") == 0) {
        char command[256];
        snprintf(command, sizeof(command), "stty rows %d cols %d 2>/dev/null; exec bash -i", lines, columns);
        fprintf(script, "if command -v script >/dev/null 2>&1; then\n");
        fprintf(script, "  script -q -f -c ");
        shell_quote(script, command);
        fprintf(script, " /dev/null < \"$stdin_source\" > ");
        shell_quote(script, out_path);
        fprintf(script, " 2>&1 &\n");
        fprintf(script, "else\n");
        fprintf(script, "  echo ");
        shell_quote(script, "Coiman IDE: Termux is missing 'script'; run: pkg install util-linux");
        fprintf(script, " > ");
        shell_quote(script, err_path);
        fprintf(script, "\n  bash -i < \"$stdin_source\" >> ");
        shell_quote(script, out_path);
        fprintf(script, " 2>> ");
        shell_quote(script, err_path);
        fprintf(script, " &\n");
        fprintf(script, "fi\n");
    } else {
        fprintf(script, "command -v ");
        shell_quote(script, tool);
        fprintf(script, " >/dev/null 2>&1 || { echo ");
        char missing[512];
        snprintf(
            missing,
            sizeof(missing),
            "Coiman IDE: '%s' is not installed in Termux. Install it with: pkg install %s",
            tool,
            tool);
        shell_quote(script, missing);
        fprintf(script, " > ");
        shell_quote(script, err_path);
        fprintf(script, "; echo 127 > ");
        shell_quote(script, exit_path);
        fprintf(script, "; exit 127; }\n");
        shell_quote(script, tool);
        for (int i = 1; i < argc; i++) {
            fputc(' ', script);
            shell_quote(script, argv[i]);
        }
        fprintf(script, " < \"$stdin_source\" > ");
        shell_quote(script, out_path);
        fprintf(script, " 2> ");
        shell_quote(script, err_path);
        fprintf(script, " &\n");
    }
    fprintf(script, "child=$!\n");
    fprintf(script, "(\n");
    fprintf(script, "  while :; do\n");
    fprintf(script, "    if [ -e ");
    shell_quote(script, kill_path);
    fprintf(script, " ]; then kill -TERM \"$child\" 2>/dev/null; break; fi\n");
    fprintf(script, "    now=$(date +%%s)\n");
    fprintf(script, "    beat=$(cat ");
    shell_quote(script, alive_path);
    fprintf(script, " 2>/dev/null || echo 0)\n");
    fprintf(script, "    beat=${beat//[^0-9]/}\n");
    fprintf(script, "    if [ -n \"$beat\" ] && [ $((now - beat)) -gt 20 ]; then kill -TERM \"$child\" 2>/dev/null; break; fi\n");
    fprintf(script, "    sleep 1\n");
    fprintf(script, "  done\n");
    fprintf(script, ") &\n");
    fprintf(script, "watchdog=$!\n");
    fprintf(script, "wait \"$child\"; rc=$?\n");
    fprintf(script, "kill \"$watchdog\" 2>/dev/null\n");
    fprintf(script, "[ -n \"$feeder\" ] && kill \"$feeder\" 2>/dev/null\n");
    fprintf(script, "rm -f \"$fifo\"\n");
    fprintf(script, "echo \"$rc\" > ");
    shell_quote(script, exit_path);
    fprintf(script, "\n");
    fclose(script);
    chmod(script_path, 0755);

    /* Publish the request atomically: the broker only sees complete directories. */
    char path[PATH_MAX_LEN];
    snprintf(path, sizeof(path), "%s/script", temp_request);
    if (write_file(path, script_path, strlen(script_path)) != 0) {
        fail("cannot write the Termux request: %s", strerror(errno));
        return 127;
    }
    snprintf(path, sizeof(path), "%s/workdir", temp_request);
    write_file(path, workdir, strlen(workdir));
    snprintf(path, sizeof(path), "%s/label", temp_request);
    char label[512];
    if (strcmp(tool, "__terminal__") == 0)
        snprintf(label, sizeof(label), "Coiman IDE: terminal");
    else
        snprintf(label, sizeof(label), "Coiman IDE: %s", tool);
    write_file(path, label, strlen(label));
    snprintf(path, sizeof(path), "%s/result", temp_request);
    write_file(path, result_dir, strlen(result_dir));
    if (rename(temp_request, request) != 0) {
        fail("cannot publish the Termux request: %s", strerror(errno));
        return 127;
    }

    signal(SIGTERM, on_signal);
    signal(SIGINT, on_signal);
    signal(SIGHUP, on_signal);

    pthread_t input_thread;
    struct input_args input = { in_path, in_done_path };
    pthread_create(&input_thread, NULL, forward_stdin, &input);

    pthread_t out_thread, err_thread, beat_thread;
    struct tail_args out_args = { out_path, STDOUT_FILENO, exit_path, result_exit };
    struct tail_args err_args = { err_path, STDERR_FILENO, exit_path, result_exit };
    pthread_create(&out_thread, NULL, tail_file, &out_args);
    pthread_create(&err_thread, NULL, tail_file, &err_args);
    pthread_create(&beat_thread, NULL, heartbeat, NULL);

    long long timeout_ms = 45000;
    const char *timeout_env = getenv("COIMAN_TERMUX_START_TIMEOUT_MS");
    if (timeout_env) timeout_ms = strtoll(timeout_env, NULL, 10);
    if (timeout_ms < 5000) timeout_ms = 5000;
    long long deadline = now_ms() + timeout_ms;

    int exit_code = -1;
    for (;;) {
        if (g_stop) {
            exit_code = 143;
            break;
        }
        if (file_exists(exit_path)) {
            exit_code = read_exit_code(exit_path);
            break;
        }
        if (file_exists(result_exit)) {
            exit_code = read_exit_code(result_exit);
            char message[2048];
            if (read_file(result_error, message, sizeof(message)) > 0 && strncmp(message, "-1", 2) != 0) {
                char *newline = strchr(message, '\n');
                if (newline) *newline = '\0';
                fail("%s", message);
            }
            break;
        }
        if (file_exists(broker_error)) {
            char message[2048];
            if (read_file(broker_error, message, sizeof(message)) > 0) fail("%s", message);
            exit_code = 126;
            break;
        }
        if (!file_exists(started_path) && now_ms() > deadline) {
            fail(
                "Termux did not start the command. Check that Termux is installed, that its "
                "allow-external-apps setting is enabled, and that it has storage access "
                "(run 'termux-setup-storage' in Termux).");
            exit_code = 124;
            break;
        }
        sleep_ms(10);
    }
    if (exit_code < 0) exit_code = 1;
    g_remote_done = 1;
    touch_file(alive_path);

    pthread_join(out_thread, NULL);
    pthread_join(err_thread, NULL);
    /* The input and heartbeat threads are not joined: they only touch this process's own pipe
     * and files, and a thread blocked in read() dies with the process when main returns. bionic
     * does not provide pthread_cancel, so nothing is signalled to them. */

    /* The request directory belongs to this process: the broker read and deleted it before
     * starting Termux, and a copy left behind (Termux never started) must not run later. */
    remove_tree(request);
    /* The scratch tree is dead weight in shared storage once the command has ended. A short pause
     * lets the service finish closing its own files. */
    sleep_ms(200);
    remove_tree(scratch);
    return exit_code;
}
