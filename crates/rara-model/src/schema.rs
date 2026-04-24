// @generated automatically by Diesel CLI.

diesel::table! {
    channel_binding (channel_type, account, chat_id) {
        channel_type -> Text,
        account -> Text,
        chat_id -> Text,
        session_key -> Text,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    chat_session (key) {
        key -> Nullable<Text>,
        title -> Nullable<Text>,
        model -> Nullable<Text>,
        system_prompt -> Nullable<Text>,
        message_count -> Integer,
        preview -> Nullable<Text>,
        metadata -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    coding_task (id) {
        id -> Text,
        status -> Integer,
        agent_type -> Integer,
        repo_url -> Text,
        branch -> Text,
        prompt -> Text,
        pr_url -> Nullable<Text>,
        pr_number -> Nullable<Integer>,
        session_key -> Nullable<Text>,
        tmux_session -> Text,
        workspace_path -> Text,
        output -> Text,
        exit_code -> Nullable<Integer>,
        error -> Nullable<Text>,
        created_at -> Text,
        started_at -> Nullable<Text>,
        completed_at -> Nullable<Text>,
    }
}

diesel::table! {
    credential_store (service, account) {
        service -> Text,
        account -> Text,
        value -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    data_feed_events (id) {
        id -> Text,
        source_name -> Text,
        event_type -> Text,
        tags -> Text,
        payload -> Text,
        received_at -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    data_feeds (id) {
        id -> Text,
        name -> Text,
        feed_type -> Text,
        tags -> Text,
        transport -> Text,
        auth -> Nullable<Text>,
        enabled -> Integer,
        status -> Text,
        last_error -> Nullable<Text>,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    execution_traces (id) {
        id -> Text,
        session_id -> Text,
        trace_data -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    feed_read_cursors (subscriber_id, source_name) {
        subscriber_id -> Text,
        source_name -> Text,
        last_read_id -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    kernel_audit_events (id) {
        id -> Text,
        timestamp -> Text,
        agent_id -> Text,
        session_id -> Text,
        user_id -> Text,
        event_type -> Text,
        event_data -> Text,
        details -> Text,
        created_at -> Text,
    }
}

diesel::table! {
    kernel_outbox (id) {
        id -> Text,
        channel_type -> Text,
        target -> Text,
        payload -> Text,
        status -> Integer,
        created_at -> Text,
        delivered_at -> Nullable<Text>,
    }
}

diesel::table! {
    kernel_users (id) {
        id -> Text,
        name -> Text,
        role -> Integer,
        permissions -> Text,
        enabled -> Integer,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    kv_table (key) {
        key -> Text,
        value -> Nullable<Text>,
    }
}

diesel::table! {
    memory_items (id) {
        id -> Nullable<Integer>,
        username -> Text,
        content -> Text,
        memory_type -> Text,
        category -> Text,
        source_tape -> Nullable<Text>,
        source_entry_id -> Nullable<Integer>,
        embedding -> Nullable<Binary>,
        created_at -> Text,
        updated_at -> Text,
    }
}

diesel::table! {
    skill_cache (name) {
        name -> Nullable<Text>,
        description -> Text,
        homepage -> Nullable<Text>,
        license -> Nullable<Text>,
        compatibility -> Nullable<Text>,
        allowed_tools -> Text,
        dockerfile -> Nullable<Text>,
        requires -> Text,
        path -> Text,
        source -> Integer,
        content_hash -> Text,
        cached_at -> Text,
    }
}

diesel::table! {
    tape_fts_meta (tape_name) {
        tape_name -> Nullable<Text>,
        last_indexed_id -> Integer,
    }
}

diesel::joinable!(channel_binding -> chat_session (session_key));

diesel::allow_tables_to_appear_in_same_query!(
    channel_binding,
    chat_session,
    coding_task,
    credential_store,
    data_feed_events,
    data_feeds,
    execution_traces,
    feed_read_cursors,
    kernel_audit_events,
    kernel_outbox,
    kernel_users,
    kv_table,
    memory_items,
    skill_cache,
    tape_fts_meta,
);
