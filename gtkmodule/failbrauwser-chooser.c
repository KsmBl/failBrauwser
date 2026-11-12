/* failBrauwser's open and save dialogs for GTK 3 applications.
 *
 * GTK 3 applications that build GTK's own file dialog (GtkFileChooserDialog; xfce4-terminal's
 * "Save Contents…", most older applications, and GtkFileChooserNative outside a sandbox)
 * never ask the desktop portal, so the portal's file chooser — failBrauwser — is not used for
 * them. Loaded into them through GTK3_MODULES (GTK 2 does not read it), this module stops
 * such a dialog from showing, asks the portal instead with the dialog's title, button,
 * file name, folder and filters, puts the answer into the dialog and answers with the
 * application's own button, exactly as if the user had picked the file there.
 *
 * Where the portal cannot be asked, the GTK dialog shows as before. Portal backends and
 * failBrauwser itself are left alone (a portal backend asking the portal would ask itself).
 * FAILBRAUWSER_NO_CHOOSER=1 turns it off for one application.
 */
#include <gtk/gtk.h>
#include <string.h>

#define PENDING "failbrauwser-chooser-pending"
#define PASS "failbrauwser-chooser-pass"

static void (*parent_show) (GtkWidget *widget);

typedef struct {
  GtkFileChooserDialog *dialog;
  /* The name of the file type picked in the portal's dialog. */
  char *filter;
  GDBusConnection *bus;
  guint subscription;
  char *handle;
  GStrv uris;
  guint wait_left;
} Request;

static void
request_free (Request *r)
{
  if (r->subscription)
    g_dbus_connection_signal_unsubscribe (r->bus, r->subscription);
  g_object_set_data (G_OBJECT (r->dialog), PENDING, NULL);
  g_object_unref (r->dialog);
  g_object_unref (r->bus);
  g_free (r->handle);
  g_free (r->filter);
  g_strfreev (r->uris);
  g_free (r);
}

/* The response of the application's button for picking (Save, Open, …), or for cancelling. */
static int
response_of (GtkDialog *dialog, gboolean accept)
{
  static const int accepting[] = { GTK_RESPONSE_ACCEPT, GTK_RESPONSE_OK, GTK_RESPONSE_YES, GTK_RESPONSE_APPLY };
  static const int cancelling[] = { GTK_RESPONSE_CANCEL, GTK_RESPONSE_CLOSE, GTK_RESPONSE_NO };
  const int *ids = accept ? accepting : cancelling;
  guint n = accept ? G_N_ELEMENTS (accepting) : G_N_ELEMENTS (cancelling);
  for (guint i = 0; i < n; i++)
    if (gtk_dialog_get_widget_for_response (dialog, ids[i]))
      return ids[i];
  return accept ? GTK_RESPONSE_ACCEPT : GTK_RESPONSE_DELETE_EVENT;
}

/* The text of the application's button for picking, for the portal's dialog. */
static const char *
accept_label (GtkDialog *dialog)
{
  GtkWidget *b = gtk_dialog_get_widget_for_response (dialog, response_of (dialog, TRUE));
  return GTK_IS_BUTTON (b) ? gtk_button_get_label (GTK_BUTTON (b)) : NULL;
}

/* Shows the GTK dialog itself after all. */
static void
show_gtk_dialog (GtkFileChooserDialog *dialog)
{
  g_object_set_data (G_OBJECT (dialog), PASS, GINT_TO_POINTER (1));
  gtk_widget_show (GTK_WIDGET (dialog));
}

/* The dialog's list of files: the tree view with a column of files; `column` is set to it. */
static GtkTreeView *
file_list (GtkWidget *w, int *column)
{
  if (GTK_IS_TREE_VIEW (w))
    {
      GtkTreeModel *m = gtk_tree_view_get_model (GTK_TREE_VIEW (w));
      for (int i = 0; m && i < gtk_tree_model_get_n_columns (m); i++)
        if (g_type_is_a (gtk_tree_model_get_column_type (m, i), G_TYPE_FILE))
          {
            *column = i;
            return GTK_TREE_VIEW (w);
          }
      return NULL;
    }
  if (!GTK_IS_CONTAINER (w))
    return NULL;
  GtkTreeView *found = NULL;
  GList *children = gtk_container_get_children (GTK_CONTAINER (w));
  for (GList *c = children; c && !found; c = c->next)
    found = file_list (c->data, column);
  g_list_free (children);
  return found;
}

/* Selects the picked files in the dialog's list once GTK has listed their folder (its own
 * select function keeps only the last of several in a dialog not shown). */
static gboolean
select_in_list (Request *r)
{
  int column = 0;
  GtkTreeView *view = file_list (GTK_WIDGET (r->dialog), &column);
  if (!view)
    return FALSE;
  GtkTreeModel *model = gtk_tree_view_get_model (view);
  guint n = g_strv_length (r->uris);
  GtkTreeIter *rows = g_new0 (GtkTreeIter, n);
  guint found = 0;
  GtkTreeIter it;
  for (gboolean ok = gtk_tree_model_get_iter_first (model, &it); ok; ok = gtk_tree_model_iter_next (model, &it))
    {
      g_autoptr (GFile) file = NULL;
      gtk_tree_model_get (model, &it, column, &file, -1);
      for (guint i = 0; file && i < n; i++)
        {
          g_autoptr (GFile) want = g_file_new_for_uri (r->uris[i]);
          if (g_file_equal (file, want))
            {
              rows[i] = it;
              found++;
            }
        }
    }
  if (found == n)
    {
      GtkTreeSelection *sel = gtk_tree_view_get_selection (view);
      gtk_tree_selection_unselect_all (sel);
      for (guint i = 0; i < n; i++)
        gtk_tree_selection_select_iter (sel, &rows[i]);
    }
  g_free (rows);
  return found == n;
}

/* Whether the dialog holds what was picked: GTK changes folders and lists them only once it
 * has looked them up. */
static gboolean
holds_answer (Request *r)
{
  GtkFileChooser *chooser = GTK_FILE_CHOOSER (r->dialog);
  if (gtk_file_chooser_get_action (chooser) != GTK_FILE_CHOOSER_ACTION_OPEN)
    {
      g_autoptr (GFile) got = gtk_file_chooser_get_file (chooser);
      g_autoptr (GFile) want = g_file_new_for_uri (r->uris[0]);
      return got && g_file_equal (got, want);
    }
  if (!select_in_list (r))
    return FALSE;
  GSList *files = gtk_file_chooser_get_files (chooser);
  guint n = g_slist_length (files);
  g_slist_free_full (files, g_object_unref);
  return n == g_strv_length (r->uris);
}

/* The dialog's own check before it answers (does the folder exist, replace the file?) is
 * done: failBrauwser did it. GTK's dialog takes an answer its file chooser part requested
 * without checking again, so the request comes first; the dialog then presses its button,
 * which does nothing while the dialog is not shown. */
static void
skip_dialog_check (GtkFileChooserDialog *dialog)
{
  GtkWidget *area = gtk_dialog_get_content_area (GTK_DIALOG (dialog));
  GList *children = gtk_container_get_children (GTK_CONTAINER (area));
  for (GList *c = children; c; c = c->next)
    if (GTK_IS_FILE_CHOOSER_WIDGET (c->data) && g_signal_lookup ("response-requested", G_OBJECT_TYPE (c->data)))
      g_signal_emit_by_name (c->data, "response-requested");
  g_list_free (children);
}

static gboolean
answer_when_ready (gpointer data)
{
  Request *r = data;
  if (!holds_answer (r) && r->wait_left-- > 0)
    return G_SOURCE_CONTINUE;
  skip_dialog_check (r->dialog);
  gtk_dialog_response (GTK_DIALOG (r->dialog), response_of (GTK_DIALOG (r->dialog), TRUE));
  request_free (r);
  return G_SOURCE_REMOVE;
}

static void
answer (Request *r)
{
  GtkFileChooser *chooser = GTK_FILE_CHOOSER (r->dialog);
  GFile *file = g_file_new_for_uri (r->uris[0]);
  switch (gtk_file_chooser_get_action (chooser))
    {
    case GTK_FILE_CHOOSER_ACTION_SAVE:
      {
        GFile *folder = g_file_get_parent (file);
        char *name = g_file_get_basename (file);
        /* failBrauwser asked about replacing a file already. */
        gtk_file_chooser_set_do_overwrite_confirmation (chooser, FALSE);
        if (folder)
          gtk_file_chooser_set_current_folder_file (chooser, folder, NULL);
        gtk_file_chooser_set_current_name (chooser, name);
        g_clear_object (&folder);
        g_free (name);
        break;
      }
    case GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER:
    case GTK_FILE_CHOOSER_ACTION_CREATE_FOLDER:
      gtk_file_chooser_set_current_folder_file (chooser, file, NULL);
      break;
    default:
      {
        /* The folder is listed, then the files selected in the list (holds_answer). */
        g_autoptr (GFile) folder = g_file_get_parent (file);
        g_autoptr (GFile) current = gtk_file_chooser_get_current_folder_file (chooser);
        if (folder && !(current && g_file_equal (folder, current)))
          gtk_file_chooser_set_current_folder_file (chooser, folder, NULL);
        for (guint i = 0; r->uris[i]; i++)
          {
            g_autofree char *name = g_path_get_basename (r->uris[i]);
            if (name[0] == '.')
              g_object_set (chooser, "show-hidden", TRUE, NULL);
          }
        break;
      }
    }
  g_object_unref (file);
  if (r->filter)
    {
      GSList *filters = gtk_file_chooser_list_filters (chooser);
      for (GSList *f = filters; f; f = f->next)
        if (g_strcmp0 (gtk_file_filter_get_name (f->data), r->filter) == 0)
          gtk_file_chooser_set_filter (chooser, f->data);
      g_slist_free (filters);
    }
  /* At most five seconds for GTK to read the folder. */
  r->wait_left = 250;
  g_timeout_add (20, answer_when_ready, r);
}

static void
on_response (GDBusConnection *bus, const char *sender, const char *path, const char *iface,
             const char *signal, GVariant *params, gpointer data)
{
  Request *r = data;
  guint32 response = 2;
  g_autoptr (GVariant) results = NULL;
  g_variant_get (params, "(u@a{sv})", &response, &results);
  g_dbus_connection_signal_unsubscribe (r->bus, r->subscription);
  r->subscription = 0;
  const char **uris = NULL;
  if (response == 0)
    g_variant_lookup (results, "uris", "^a&s", &uris);
  if (response == 0 && uris && uris[0])
    {
      r->uris = g_strdupv ((char **) uris);
      g_free (uris);
      g_autoptr (GVariant) filter = g_variant_lookup_value (results, "current_filter", G_VARIANT_TYPE ("(sa(us))"));
      if (filter)
        g_variant_get_child (filter, 0, "s", &r->filter);
      answer (r);
      return;
    }
  g_free (uris);
  GtkFileChooserDialog *dialog = g_object_ref (r->dialog);
  request_free (r);
  if (response == 1)
    gtk_dialog_response (GTK_DIALOG (dialog), response_of (GTK_DIALOG (dialog), FALSE));
  else
    /* The portal could not show its dialog: GTK's instead. */
    show_gtk_dialog (dialog);
  g_object_unref (dialog);
}

static void
on_called (GObject *source, GAsyncResult *res, gpointer data)
{
  Request *r = data;
  g_autoptr (GError) error = NULL;
  g_autoptr (GVariant) ret = g_dbus_connection_call_finish (G_DBUS_CONNECTION (source), res, &error);
  if (ret)
    {
      const char *handle = NULL;
      g_variant_get (ret, "(&o)", &handle);
      /* Older portals choose the request path themselves. */
      if (g_strcmp0 (handle, r->handle) != 0)
        {
          g_dbus_connection_signal_unsubscribe (r->bus, r->subscription);
          g_free (r->handle);
          r->handle = g_strdup (handle);
          r->subscription = g_dbus_connection_signal_subscribe (r->bus, "org.freedesktop.portal.Desktop",
                                                                "org.freedesktop.portal.Request", "Response",
                                                                r->handle, NULL, G_DBUS_SIGNAL_FLAGS_NO_MATCH_RULE,
                                                                on_response, r, NULL);
        }
      return;
    }
  g_message ("failBrauwser: the desktop portal was not asked (%s); showing GTK's dialog", error->message);
  GtkFileChooserDialog *dialog = g_object_ref (r->dialog);
  request_free (r);
  show_gtk_dialog (dialog);
  g_object_unref (dialog);
}

/* Asks the desktop portal for what the dialog would ask; FALSE when it cannot be asked. */
static gboolean
ask_portal (GtkFileChooserDialog *dialog)
{
  GtkFileChooser *chooser = GTK_FILE_CHOOSER (dialog);
  g_autoptr (GError) error = NULL;
  GDBusConnection *bus = g_bus_get_sync (G_BUS_TYPE_SESSION, NULL, &error);
  if (!bus)
    {
      g_object_set_data (G_OBJECT (dialog), PENDING, NULL);
      return FALSE;
    }

  Request *r = g_new0 (Request, 1);
  r->dialog = g_object_ref (dialog);
  r->bus = bus;
  g_autofree char *sender = g_strdup (g_dbus_connection_get_unique_name (bus) + 1);
  for (char *c = sender; *c; c++)
    if (*c == '.')
      *c = '_';
  g_autofree char *token = g_strdup_printf ("failbrauwser%u", g_random_int ());
  r->handle = g_strdup_printf ("/org/freedesktop/portal/desktop/request/%s/%s", sender, token);
  /* Before asking, so that a quick answer is not missed. */
  r->subscription = g_dbus_connection_signal_subscribe (bus, "org.freedesktop.portal.Desktop",
                                                        "org.freedesktop.portal.Request", "Response",
                                                        r->handle, NULL, G_DBUS_SIGNAL_FLAGS_NO_MATCH_RULE,
                                                        on_response, r, NULL);

  GtkFileChooserAction action = gtk_file_chooser_get_action (chooser);
  gboolean save = action == GTK_FILE_CHOOSER_ACTION_SAVE;
  GVariantBuilder opts;
  g_variant_builder_init (&opts, G_VARIANT_TYPE_VARDICT);
  g_variant_builder_add (&opts, "{sv}", "handle_token", g_variant_new_string (token));
  g_variant_builder_add (&opts, "{sv}", "modal", g_variant_new_boolean (gtk_window_get_modal (GTK_WINDOW (dialog))));
  const char *label = accept_label (GTK_DIALOG (dialog));
  if (label)
    g_variant_builder_add (&opts, "{sv}", "accept_label", g_variant_new_string (label));
  if (action == GTK_FILE_CHOOSER_ACTION_SELECT_FOLDER || action == GTK_FILE_CHOOSER_ACTION_CREATE_FOLDER)
    g_variant_builder_add (&opts, "{sv}", "directory", g_variant_new_boolean (TRUE));
  if (!save)
    g_variant_builder_add (&opts, "{sv}", "multiple", g_variant_new_boolean (gtk_file_chooser_get_select_multiple (chooser)));
  if (save)
    {
      g_autofree char *name = gtk_file_chooser_get_current_name (chooser);
      if (name && *name)
        g_variant_builder_add (&opts, "{sv}", "current_name", g_variant_new_string (name));
    }
  g_autofree char *folder = gtk_file_chooser_get_current_folder (chooser);
  if (folder)
    g_variant_builder_add (&opts, "{sv}", "current_folder", g_variant_new_bytestring (folder));
  GSList *filters = gtk_file_chooser_list_filters (chooser);
  if (filters)
    {
      GVariantBuilder list;
      g_variant_builder_init (&list, G_VARIANT_TYPE ("a(sa(us))"));
      for (GSList *f = filters; f; f = f->next)
        g_variant_builder_add_value (&list, gtk_file_filter_to_gvariant (f->data));
      g_variant_builder_add (&opts, "{sv}", "filters", g_variant_builder_end (&list));
      g_slist_free (filters);
    }
  GtkFileFilter *current = gtk_file_chooser_get_filter (chooser);
  if (current)
    g_variant_builder_add (&opts, "{sv}", "current_filter", gtk_file_filter_to_gvariant (current));

  const char *title = gtk_window_get_title (GTK_WINDOW (dialog));
  g_dbus_connection_call (bus, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop",
                          "org.freedesktop.portal.FileChooser", save ? "SaveFile" : "OpenFile",
                          g_variant_new ("(ssa{sv})", "", title ? title : "", &opts),
                          G_VARIANT_TYPE ("(o)"), G_DBUS_CALL_FLAGS_NONE, -1, NULL, on_called, r);
  return TRUE;
}

/* GTK takes a moment to turn a folder the application set into the dialog's current folder
 * (it looks the folder up first): the portal is asked once it has, or after 150 ms when the
 * application set none. */
static gboolean
ask_when_ready (gpointer data)
{
  GtkFileChooserDialog *dialog = data;
  int left = GPOINTER_TO_INT (g_object_get_data (G_OBJECT (dialog), PENDING)) - 1;
  g_autofree char *folder = gtk_file_chooser_get_current_folder (GTK_FILE_CHOOSER (dialog));
  if (!folder && left > 0)
    {
      g_object_set_data (G_OBJECT (dialog), PENDING, GINT_TO_POINTER (left));
      return G_SOURCE_CONTINUE;
    }
  /* From here on the request holds the dialog as pending. */
  g_object_set_data (G_OBJECT (dialog), PENDING, GINT_TO_POINTER (-1));
  if (!ask_portal (dialog))
    show_gtk_dialog (dialog);
  return G_SOURCE_REMOVE;
}

static void
chooser_show (GtkWidget *widget)
{
  GObject *o = G_OBJECT (widget);
  if (g_object_get_data (o, PASS))
    {
      g_object_set_data (o, PASS, NULL);
      parent_show (widget);
      return;
    }
  /* Asked already: the answer comes. */
  if (g_object_get_data (o, PENDING))
    return;
  /* At most 30 checks, 5 ms apart. */
  g_object_set_data (o, PENDING, GINT_TO_POINTER (31));
  g_timeout_add_full (G_PRIORITY_DEFAULT, 5, ask_when_ready, g_object_ref (widget), g_object_unref);
}

/* Whether this program's dialogs are left as they are. */
static gboolean
left_alone (void)
{
  const char *off = g_getenv ("FAILBRAUWSER_NO_CHOOSER");
  if (off && *off && strcmp (off, "0") != 0)
    return TRUE;
  const char *prg = g_get_prgname ();
  if (!prg)
    return FALSE;
  g_autofree char *base = g_path_get_basename (prg);
  return strcmp (base, "failbrauwser") == 0 || g_str_has_prefix (base, "xdg-desktop-portal");
}

G_MODULE_EXPORT void
gtk_module_init (gint *argc, gchar ***argv)
{
  if (gtk_get_major_version () != 3 || left_alone ())
    return;
  /* Once, also when GTK loads it twice (GTK3_MODULES and the settings, other places). */
  GQuark patched = g_quark_from_static_string ("failbrauwser-chooser-patched");
  if (g_type_get_qdata (GTK_TYPE_FILE_CHOOSER_DIALOG, patched))
    return;
  g_type_set_qdata (GTK_TYPE_FILE_CHOOSER_DIALOG, patched, GINT_TO_POINTER (1));
  GtkWidgetClass *klass = g_type_class_ref (GTK_TYPE_FILE_CHOOSER_DIALOG);
  parent_show = klass->show;
  klass->show = chooser_show;
}

G_MODULE_EXPORT const gchar *
g_module_check_init (GModule *module)
{
  /* Patched classes outlive the module: it is never unloaded. */
  g_module_make_resident (module);
  return NULL;
}
