/* =============================================================================
 * pam_test_runner.c — Minimal Deterministic PAM Authentication Test Harness
 * =============================================================================
 * Compiles cleanly on any distribution with:
 *   gcc pam_test_runner.c -lpam -o pam_test_runner
 *
 * Usage:
 *   pam_test_runner <service> <user> [password]
 *
 * Behavior:
 *   - If password is provided as 3rd argument, it will be supplied to PAM conv.
 *   - If password is not provided, conv will return PAM_CONV_ERR on any prompt.
 *     This cleanly asserts that facial authentication succeeds WITHOUT any
 *     password prompt (non-interactive authentication).
 *
 * Exit Code:
 *   Returns the exact PAM status code (PAM_SUCCESS = 0).
 * ============================================================================= */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <security/pam_appl.h>

static const char *g_password = NULL;
static int g_prompt_count = 0;

static int test_conv(int num_msg, const struct pam_message **msg,
                     struct pam_response **resp, void *appdata_ptr) {
    (void)appdata_ptr;
    if (num_msg <= 0 || num_msg > 32) {
        return PAM_CONV_ERR;
    }

    struct pam_response *reply = calloc((size_t)num_msg, sizeof(struct pam_response));
    if (!reply) {
        return PAM_BUF_ERR;
    }

    for (int i = 0; i < num_msg; ++i) {
        int style = msg[i]->msg_style;
        if (style == PAM_PROMPT_ECHO_OFF || style == PAM_PROMPT_ECHO_ON) {
            g_prompt_count++;
            if (!g_password) {
                /* Non-interactive expectation violated */
                for (int j = 0; j < i; ++j) {
                    free(reply[j].resp);
                }
                free(reply);
                return PAM_CONV_ERR;
            }
            reply[i].resp = strdup(g_password);
            reply[i].resp_retcode = 0;
        } else if (style == PAM_TEXT_INFO || style == PAM_ERROR_MSG) {
            reply[i].resp = NULL;
            reply[i].resp_retcode = 0;
        } else {
            for (int j = 0; j < i; ++j) {
                free(reply[j].resp);
            }
            free(reply);
            return PAM_CONV_ERR;
        }
    }

    *resp = reply;
    return PAM_SUCCESS;
}

int main(int argc, char *argv[]) {
    if (argc < 3 || argc > 4) {
        fprintf(stderr, "Usage: %s <service> <user> [password]\n", argv[0]);
        return 1;
    }

    const char *service = argv[1];
    const char *user = argv[2];
    if (argc == 4) {
        g_password = argv[3];
    }

    struct pam_conv conv = {
        .conv = test_conv,
        .appdata_ptr = NULL,
    };

    pam_handle_t *pamh = NULL;
    int ret = pam_start(service, user, &conv, &pamh);
    if (ret != PAM_SUCCESS) {
        fprintf(stderr, "[pam_test_runner] pam_start failed: %d (%s)\n", ret, pam_strerror(pamh, ret));
        return ret;
    }

    ret = pam_authenticate(pamh, 0);
    printf("[pam_test_runner] authenticate result=%d (%s) prompts=%d\n",
           ret, pam_strerror(pamh, ret), g_prompt_count);

    int end_ret = pam_end(pamh, ret);
    if (end_ret != PAM_SUCCESS) {
        fprintf(stderr, "[pam_test_runner] pam_end warning: %d\n", end_ret);
    }

    return ret;
}
