use std::{str::FromStr, time::Duration};

use adw::prelude::*;
use adw::subclass::prelude::*;
use btfm_api_structs::{Clip, Clips};
use gtk::{
    gio::{self, prelude::SettingsExtManual},
    glib, Button, Label, ScrolledWindow, SelectionMode, StackPage,
};

glib::wrapper! {
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends gtk::ApplicationWindow, gtk::Window, gtk::Widget, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    pub fn new<P: glib::IsA<gtk::Application>>(application: &P) -> Self {
        let window = glib::Object::builder()
            .property("application", application)
            .build();
        window
    }

    fn setup_settings(&self) {
        let settings = gio::Settings::new(crate::APP_ID);
        self.imp()
            .settings
            .set(settings)
            .expect("`settings` should not be set before calling `setup_settings`.");
    }

    fn settings(&self) -> &gio::Settings {
        self.imp()
            .settings
            .get()
            .expect("`settings` should be set in `setup_settings`.")
    }

    fn setup_actions(&self) {
        let action_connect_server = gio::SimpleAction::new("connect-to-server", None);
        action_connect_server.connect_activate(glib::clone!(@weak self as window => move |_, _| {
            window.load_clips();
        }));
        self.add_action(&action_connect_server)
    }

    fn populate_creds(&self) {
        let username: String = self.settings().get("username");
        let password: String = self.settings().get("password");
        let server_url: String = self.settings().get("server-url");
        self.imp().server_url.set_text(&server_url);
        self.imp().username.set_text(&username);
        self.imp().password.set_text(&password);
    }

    fn load_clips(&self) {
        let server_url = self.imp().server_url.text().to_string();
        let username = self.imp().username.text().to_string();
        let password = self.imp().password.text().to_string();

        self.settings()
            .set_string("server-url", server_url.as_str())
            .unwrap();
        self.settings()
            .set_string("username", username.as_str())
            .unwrap();
        self.settings()
            .set_string("password", password.as_str())
            .unwrap();

        // TODO: don't block the main thread, handle errors

        let server_url = url::Url::from_str(&server_url)
            .unwrap()
            .join("/v1/clips/")
            .unwrap();
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap();
        let response = client
            .get(server_url)
            .basic_auth(username, Some(password))
            .send()
            .unwrap();
        let clips = response.json::<Clips>().unwrap();

        // TODO wire up selecting a clip by label to display the details.
        for clip in clips.clips {
            let ulid = clip.ulid.to_string();
            let clip_details_list_box = gtk::ListBox::builder()
                .selection_mode(SelectionMode::None)
                .build();

            clip_details_list_box.append(
                &adw::ActionRow::builder()
                    .title(&ulid)
                    .subtitle("ID")
                    .build(),
            );
            clip_details_list_box.append(
                &adw::ActionRow::builder()
                    .title(&clip.created_on.to_string())
                    .icon_name("clock-symbolic")
                    .subtitle("Created on")
                    .build(),
            );
            clip_details_list_box.append(
                &adw::ActionRow::builder()
                    .title(&clip.last_played.to_string())
                    .icon_name("clock-symbolic")
                    .subtitle("Last played")
                    .build(),
            );
            clip_details_list_box.append(
                &adw::ActionRow::builder()
                    .title(&clip.plays.to_string())
                    .icon_name("play-symbolic")
                    .subtitle("Play count")
                    .build(),
            );
            clip_details_list_box.append(
                &adw::ActionRow::builder()
                    .title(&clip.description)
                    .icon_name("notepad-symbolic")
                    .subtitle("Description")
                    .build(),
            );
            if let Some(phrases) = clip.phrases {
                let phrase_list_box = gtk::ListBox::builder()
                    .selection_mode(SelectionMode::None)
                    .tooltip_text("Phrases")
                    .build();
                for phrase in phrases.phrases.iter() {
                    phrase_list_box.append(
                        &adw::ActionRow::builder()
                            .title(&phrase.phrase)
                            .icon_name("notepad-symbolic")
                            .subtitle(phrase.ulid.to_string().as_str())
                            .build(),
                    )
                }
                let scroll_window = ScrolledWindow::builder()
                    .vexpand(true)
                    .hexpand_set(false)
                    .child(&phrase_list_box)
                    .build();
                clip_details_list_box.append(&scroll_window);
            }

            let scroll_window = ScrolledWindow::builder()
                .vexpand(true)
                .hexpand_set(false)
                .child(&clip_details_list_box)
                .build();
            self.imp()
                .clips_details
                .add_named(&scroll_window, Some(&ulid));

            let clip_sidebar_entry = adw::ActionRow::builder()
                .title(&ulid)
                .subtitle("ID")
                .build();
            clip_sidebar_entry.connect_has_focus_notify(
                glib::clone!(@weak self as window => move |_action_row| {
                    window.imp().clips_details.set_visible_child_name(&ulid);
                }),
            );
            self.imp().clips_list.append(&clip_sidebar_entry);
        }
        self.imp().stack.set_visible_child_name("clip_stack_page");
    }
}

mod imp {
    use adw::prelude::*;
    use adw::subclass::prelude::*;
    use adw::Leaflet;
    use glib::subclass::InitializingObject;
    use gtk::{glib, Button, CompositeTemplate, Entry, ListBox, PasswordEntry, Stack};
    use once_cell::sync::OnceCell;

    #[derive(CompositeTemplate, Default)]
    #[template(resource = "/org/jcline/btfm/window.ui")]
    pub struct Window {
        pub settings: OnceCell<gtk::gio::Settings>,
        #[template_child]
        pub clips_list: TemplateChild<ListBox>,
        #[template_child]
        pub clips_details: TemplateChild<Stack>,
        #[template_child]
        pub leaflet: TemplateChild<Leaflet>,
        #[template_child]
        pub stack: TemplateChild<Stack>,
        #[template_child]
        pub back_button: TemplateChild<Button>,
        #[template_child]
        pub server_url: TemplateChild<Entry>,
        #[template_child]
        pub username: TemplateChild<Entry>,
        #[template_child]
        pub password: TemplateChild<PasswordEntry>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "BtfmWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;

        fn class_init(class: &mut Self::Class) {
            class.bind_template();
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.setup_settings();
            obj.setup_actions();
            obj.populate_creds();
        }
    }

    // Trait shared by all widgets
    impl WidgetImpl for Window {}
    // Trait shared by all windows
    impl WindowImpl for Window {}
    // Trait shared by all application windows
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}
