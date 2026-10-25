/* slot's side of rcheevos: one rc_client for the whole process, flattened into plain C calls
 * so the Rust side needs no struct layouts. Every call is made with slot's lock held. */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "rc_client.h"
#include "rc_api_runtime.h"
#include "rc_api_user.h"
#include "rc_api_request.h"
#include "rc_hash.h"
#include "rc_libretro.h"
#include "libretro.h"

/* Implemented in Rust (slot-ach/src/client.rs). */
extern void slot_rc_on_request(const char* url, const char* post_data, const char* content_type,
                               void* callback, void* callback_data);
extern void slot_rc_on_done(int kind, int result, const char* error);
extern void slot_rc_on_event(uint32_t type, uint32_t id, const char* title, const char* description,
                             const char* badge, uint32_t points);
extern void slot_rc_on_log(const char* message);

static rc_client_t* g_client;
static rc_libretro_memory_regions_t g_regions;
static int g_have_regions;

static uint32_t RC_CCONV read_memory(uint32_t address, uint8_t* buffer, uint32_t num_bytes, rc_client_t* client) {
  (void)client;
  if (!g_have_regions)
    return 0;
  return rc_libretro_memory_read(&g_regions, address, buffer, num_bytes);
}

static void RC_CCONV server_call(const rc_api_request_t* request, rc_client_server_callback_t callback,
                                 void* callback_data, rc_client_t* client) {
  (void)client;
  slot_rc_on_request(request->url, request->post_data ? request->post_data : "",
                     request->content_type ? request->content_type : "", (void*)callback, callback_data);
}

static void RC_CCONV on_event(const rc_client_event_t* event, rc_client_t* client) {
  (void)client;
  const rc_client_achievement_t* a = event->achievement;
  switch (event->type) {
    case RC_CLIENT_EVENT_ACHIEVEMENT_TRIGGERED:
      if (a)
        slot_rc_on_event(event->type, a->id, a->title, a->description, a->badge_name, a->points);
      break;
    case RC_CLIENT_EVENT_SERVER_ERROR:
      if (event->server_error)
        slot_rc_on_event(event->type, event->server_error->related_id, event->server_error->api,
                         event->server_error->error_message, "", 0);
      break;
    case RC_CLIENT_EVENT_GAME_COMPLETED:
    case RC_CLIENT_EVENT_DISCONNECTED:
    case RC_CLIENT_EVENT_RECONNECTED:
      slot_rc_on_event(event->type, 0, "", "", "", 0);
      break;
    default:
      break;
  }
}

static void RC_CCONV on_log(const char* message, const rc_client_t* client) {
  (void)client;
  slot_rc_on_log(message);
}

/* kind: 1 login, 2 game load. */
static void RC_CCONV login_done(int result, const char* error, rc_client_t* client, void* userdata) {
  (void)client; (void)userdata;
  slot_rc_on_done(1, result, error ? error : "");
}

static void RC_CCONV load_done(int result, const char* error, rc_client_t* client, void* userdata) {
  (void)client; (void)userdata;
  slot_rc_on_done(2, result, error ? error : "");
}

int slot_rc_create(const char* user_agent_prefix, int verbose) {
  if (g_client)
    return 1;
  g_client = rc_client_create(read_memory, server_call);
  if (!g_client)
    return 0;
  rc_client_set_event_handler(g_client, on_event);
  rc_client_enable_logging(g_client, verbose ? RC_CLIENT_LOG_LEVEL_VERBOSE : RC_CLIENT_LOG_LEVEL_WARN, on_log);
  /* Softcore only: RetroAchievements grants hardcore only to emulators it has approved. */
  rc_client_set_hardcore_enabled(g_client, 0);
  (void)user_agent_prefix;
  return 1;
}

size_t slot_rc_user_agent_clause(char* buffer, size_t size) {
  return g_client ? rc_client_get_user_agent_clause(g_client, buffer, size) : 0;
}

void slot_rc_login_password(const char* user, const char* password) {
  if (g_client)
    rc_client_begin_login_with_password(g_client, user, password, login_done, NULL);
}

void slot_rc_login_token(const char* user, const char* token) {
  if (g_client)
    rc_client_begin_login_with_token(g_client, user, token, login_done, NULL);
}

void slot_rc_logout(void) {
  if (g_client)
    rc_client_logout(g_client);
}

/* Copies the logged-in user's name and token; returns 0 if nobody is logged in. */
int slot_rc_user(char* name, size_t name_size, char* token, size_t token_size) {
  const rc_client_user_t* u = g_client ? rc_client_get_user_info(g_client) : NULL;
  if (!u)
    return 0;
  snprintf(name, name_size, "%s", u->username ? u->username : "");
  snprintf(token, token_size, "%s", u->token ? u->token : "");
  return 1;
}

void slot_rc_respond(void* callback, void* callback_data, const char* body, size_t length, int status) {
  rc_api_server_response_t response;
  memset(&response, 0, sizeof(response));
  response.body = body;
  response.body_length = length;
  response.http_status_code = status;
  ((rc_client_server_callback_t)callback)(&response, callback_data);
}

/* One descriptor of the core's memory map, as slot copied it. */
typedef struct slot_rc_desc {
  uint64_t flags;
  void* ptr;
  size_t offset;
  size_t start;
  size_t select;
  size_t disconnect;
  size_t len;
} slot_rc_desc;

static uint8_t* g_sysram;
static size_t g_sysram_len;
static uint8_t* g_saveram;
static size_t g_saveram_len;

static void RC_CCONV core_memory(uint32_t id, rc_libretro_core_memory_info_t* info) {
  info->data = NULL;
  info->size = 0;
  if (id == RETRO_MEMORY_SYSTEM_RAM) {
    info->data = g_sysram;
    info->size = g_sysram_len;
  } else if (id == RETRO_MEMORY_SAVE_RAM) {
    info->data = g_saveram;
    info->size = g_saveram_len;
  }
}

/* Points rcheevos at the running core's memory, from its memory map when it gave one. */
int slot_rc_set_memory(const slot_rc_desc* descs, unsigned count, uint32_t console_id,
                       uint8_t* sysram, size_t sysram_len, uint8_t* saveram, size_t saveram_len) {
  struct retro_memory_descriptor* copy = NULL;
  struct retro_memory_map map;
  unsigned i;
  int ok;

  if (g_have_regions) {
    rc_libretro_memory_destroy(&g_regions);
    g_have_regions = 0;
  }
  g_sysram = sysram;
  g_sysram_len = sysram_len;
  g_saveram = saveram;
  g_saveram_len = saveram_len;

  if (count) {
    copy = (struct retro_memory_descriptor*)calloc(count, sizeof(*copy));
    if (!copy)
      return 0;
    for (i = 0; i < count; i++) {
      copy[i].flags = descs[i].flags;
      copy[i].ptr = descs[i].ptr;
      copy[i].offset = descs[i].offset;
      copy[i].start = descs[i].start;
      copy[i].select = descs[i].select;
      copy[i].disconnect = descs[i].disconnect;
      copy[i].len = descs[i].len;
    }
    map.descriptors = copy;
    map.num_descriptors = count;
  }
  ok = rc_libretro_memory_init(&g_regions, count ? &map : NULL, core_memory, console_id);
  free(copy);
  g_have_regions = ok;
  return ok;
}

void slot_rc_clear_memory(void) {
  if (g_have_regions)
    rc_libretro_memory_destroy(&g_regions);
  g_have_regions = 0;
  g_sysram = g_saveram = NULL;
  g_sysram_len = g_saveram_len = 0;
}

void slot_rc_load(uint32_t console_id, const char* path) {
  if (g_client)
    rc_client_begin_identify_and_load_game(g_client, console_id, path, NULL, 0, load_done, NULL);
}

/* RetroAchievements' hash of a ROM file. Touches no client state, so any thread may call it. */
int slot_rc_hash(uint32_t console_id, const char* path, char hash[33]) {
  return rc_hash_generate_from_file(hash, console_id, path);
}

void slot_rc_load_hash(const char* hash) {
  if (g_client)
    rc_client_begin_load_game(g_client, hash, load_done, NULL);
}

void slot_rc_unload(void) {
  if (g_client)
    rc_client_unload_game(g_client);
}

void slot_rc_frame(void) {
  if (g_client)
    rc_client_do_frame(g_client);
}

void slot_rc_idle(void) {
  if (g_client)
    rc_client_idle(g_client);
}

void slot_rc_reset(void) {
  if (g_client)
    rc_client_reset(g_client);
}

/* The loaded game: id, title, hash, and how many of its achievements are unlocked. */
int slot_rc_game(uint32_t* id, char* title, size_t title_size, char* hash, size_t hash_size,
                 char* badge, size_t badge_size, uint32_t* unlocked, uint32_t* total) {
  const rc_client_game_t* g = g_client ? rc_client_get_game_info(g_client) : NULL;
  rc_client_user_game_summary_t summary;
  if (!g || !g->id)
    return 0;
  *id = g->id;
  snprintf(title, title_size, "%s", g->title ? g->title : "");
  snprintf(hash, hash_size, "%s", g->hash ? g->hash : "");
  snprintf(badge, badge_size, "%s", g->badge_name ? g->badge_name : "");
  memset(&summary, 0, sizeof(summary));
  rc_client_get_user_game_summary(g_client, &summary);
  *unlocked = summary.num_unlocked_achievements;
  *total = summary.num_core_achievements;
  return 1;
}

/* Calls back once per achievement of the loaded game with its badge name, so slot can fetch
 * the images while online. */
extern void slot_rc_on_badge(const char* badge);

void slot_rc_each_badge(void) {
  rc_client_achievement_list_t* list;
  uint32_t b, a;
  if (!g_client)
    return;
  list = rc_client_create_achievement_list(g_client, RC_CLIENT_ACHIEVEMENT_CATEGORY_CORE,
                                           RC_CLIENT_ACHIEVEMENT_LIST_GROUPING_LOCK_STATE);
  if (!list)
    return;
  for (b = 0; b < list->num_buckets; b++)
    for (a = 0; a < list->buckets[b].num_achievements; a++)
      slot_rc_on_badge(list->buckets[b].achievements[a]->badge_name);
  rc_client_destroy_achievement_list(list);
}

/* Builds the award request for an unlock earned offline, `seconds` ago. */
int slot_rc_award_request(const char* user, const char* token, uint32_t id, const char* hash,
                          uint32_t seconds, char* url, size_t url_size, char* post, size_t post_size) {
  rc_api_award_achievement_request_t params;
  rc_api_request_t request;
  int result;
  memset(&params, 0, sizeof(params));
  params.username = user;
  params.api_token = token;
  params.achievement_id = id;
  params.hardcore = 0;
  params.game_hash = hash;
  params.seconds_since_unlock = seconds;
  result = rc_api_init_award_achievement_request(&request, &params);
  if (result == RC_OK) {
    snprintf(url, url_size, "%s", request.url);
    snprintf(post, post_size, "%s", request.post_data ? request.post_data : "");
  }
  rc_api_destroy_request(&request);
  return result == RC_OK;
}

static int copy_request(int result, rc_api_request_t* request, char* url, size_t url_size,
                        char* post, size_t post_size) {
  if (result == RC_OK) {
    snprintf(url, url_size, "%s", request->url);
    snprintf(post, post_size, "%s", request->post_data ? request->post_data : "");
  }
  rc_api_destroy_request(request);
  return result == RC_OK;
}

/* The achievement-set request for a hash, as rc_client would make it when loading a game. */
int slot_rc_sets_request(const char* user, const char* token, const char* hash,
                         char* url, size_t url_size, char* post, size_t post_size) {
  rc_api_fetch_game_sets_request_t params;
  rc_api_request_t request;
  memset(&params, 0, sizeof(params));
  params.username = user;
  params.api_token = token;
  params.game_hash = hash;
  return copy_request(rc_api_init_fetch_game_sets_request(&request, &params), &request,
                      url, url_size, post, post_size);
}

/* The account's unlocks in one game. */
int slot_rc_unlocks_request(const char* user, const char* token, uint32_t game_id, int hardcore,
                            char* url, size_t url_size, char* post, size_t post_size) {
  rc_api_fetch_user_unlocks_request_t params;
  rc_api_request_t request;
  memset(&params, 0, sizeof(params));
  params.username = user;
  params.api_token = token;
  params.game_id = game_id;
  params.hardcore = hardcore ? 1 : 0;
  return copy_request(rc_api_init_fetch_user_unlocks_request(&request, &params), &request,
                      url, url_size, post, post_size);
}
