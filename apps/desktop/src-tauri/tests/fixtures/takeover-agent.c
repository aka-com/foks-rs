/* Test-only local agent. Direct mode can deliberately replace a socket to
 * simulate a competing launcher; desktop-launch mode refuses live sockets. */
#include <arpa/inet.h>
#include <fcntl.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

/* A test may build the fixture with another mode to bind a socket that the
 * desktop client refuses as unsafe. */
#ifndef SOCKET_MODE
#define SOCKET_MODE 0600
#endif

static int read_all(int fd, void *buffer, unsigned size) {
    unsigned offset = 0;
    while (offset < size) {
        int count = read(fd, (char *)buffer + offset, size - offset);
        if (count <= 0) return 0;
        offset += count;
    }
    return 1;
}

int main(int argc, char **argv) {
    if (argc < 4) return 2;
    signal(SIGPIPE, SIG_IGN);
    int launched = strcmp(argv[1], "--state-dir") == 0;
    const char *path = launched ? argv[4] : argv[1];
    int version = launched ? PROTOCOL_VERSION : atoi(argv[3]);
    struct sockaddr_un address = {0};
    address.sun_family = AF_UNIX;
    if (strlen(path) >= sizeof address.sun_path) return 3;
    strcpy(address.sun_path, path);
    if (launched) {
        int probe = socket(AF_UNIX, SOCK_STREAM, 0);
        if (connect(probe, (struct sockaddr *)&address, sizeof address) == 0) return 4;
        close(probe);
    }
    unlink(path);
    int server = socket(AF_UNIX, SOCK_STREAM, 0);
    if (bind(server, (struct sockaddr *)&address, sizeof address) != 0 ||
        chmod(path, SOCKET_MODE) != 0 || listen(server, 32) != 0) return 5;
    if (!launched) {
        int marker = open(argv[2], O_CREAT | O_WRONLY, 0600);
        if (marker < 0) return 6;
        close(marker);
    }
    for (;;) {
        int client = accept(server, NULL, NULL);
        if (client < 0) continue;
        uint32_t length;
        char request[4096];
        if (!read_all(client, &length, 4) || ntohl(length) >= sizeof request ||
            !read_all(client, request, ntohl(length))) { close(client); continue; }
        request[ntohl(length)] = '\0';
        char *id = strstr(request, "\"id\":");
        unsigned long long request_id = id ? strtoull(id + 5, NULL, 10) : 0;
        char response[512];
        int count;
        /* A Hello is answered whatever its version, naming this agent's. */
        if (strstr(request, "\"operation\":\"hello\"")) {
            count = snprintf(response, sizeof response,
                "{\"version\":%d,\"id\":%llu,\"status\":\"success\",\"value\":"
                "{\"protocol\":%d,\"supported\":{\"minimum\":%d,\"maximum\":%d},"
                "\"agent\":{\"kind\":\"foks-agent\",\"version\":\"0\"},"
                "\"capabilities\":[]}}",
                version, request_id, version, version, version);
        } else {
            count = snprintf(response, sizeof response,
                "{\"version\":%d,\"id\":%llu,\"status\":\"success\",\"value\":{\"state\":\"ready\"}}",
                version, request_id);
        }
        length = htonl(count);
        write(client, &length, 4);
        write(client, response, count);
        close(client);
    }
}
