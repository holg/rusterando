#![recursion_limit = "512"]

pub mod app;
pub mod branding;
pub mod components;
pub mod pages;
pub mod utils;

/// Explicitly register all `#[server]` items so they survive linking from a
/// downstream binary. Inventory-based auto-registration is unreliable when the
/// frontend is built as a cdylib+rlib and consumed by the server binary.
#[cfg(feature = "ssr")]
pub fn register_server_fns() {
    leptos::server_fn::axum::register_explicit::<pages::menu::ListMenu>();
    leptos::server_fn::axum::register_explicit::<pages::menu::ListAdminMenu>();
    leptos::server_fn::axum::register_explicit::<pages::admin::login::AdminLogin>();
    leptos::server_fn::axum::register_explicit::<pages::admin::login::AdminLogout>();
    leptos::server_fn::axum::register_explicit::<pages::admin::menu_admin::UpdateMenuItem>();
    leptos::server_fn::axum::register_explicit::<pages::admin::menu_admin::CreateMenuItem>();
    leptos::server_fn::axum::register_explicit::<pages::admin::extras_admin::ListExtrasAdmin>();
    leptos::server_fn::axum::register_explicit::<pages::admin::extras_admin::ListPizzaExtras>();
    leptos::server_fn::axum::register_explicit::<pages::admin::extras_admin::UpdateExtra>();
    leptos::server_fn::axum::register_explicit::<pages::admin::extras_admin::CreateExtra>();
    leptos::server_fn::axum::register_explicit::<pages::cart::GetCart>();
    leptos::server_fn::axum::register_explicit::<pages::cart::AddToCart>();
    leptos::server_fn::axum::register_explicit::<pages::cart::UpdateCartLine>();
    leptos::server_fn::axum::register_explicit::<pages::cart::ClearCart>();
    leptos::server_fn::axum::register_explicit::<pages::order::LoadCheckoutContext>();
    leptos::server_fn::axum::register_explicit::<pages::order::PlaceOrder>();
    leptos::server_fn::axum::register_explicit::<pages::order::GetOrder>();
    leptos::server_fn::axum::register_explicit::<pages::order::LookupCustomerByPhone>();
    leptos::server_fn::axum::register_explicit::<pages::order::ValidateAddress>();
    leptos::server_fn::axum::register_explicit::<pages::order::StartTour>();
    leptos::server_fn::axum::register_explicit::<pages::order::GetTour>();
    leptos::server_fn::axum::register_explicit::<pages::order::TourStopDelivered>();
    leptos::server_fn::axum::register_explicit::<pages::order::FinishTour>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::ListAdminOrders>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::UpdateOrderStatus>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::GetAdminOrder>();
    leptos::server_fn::axum::register_explicit::<pages::admin::home::LoadAdminStats>();
    leptos::server_fn::axum::register_explicit::<pages::admin::history::ListAdminHistory>();
    leptos::server_fn::axum::register_explicit::<pages::session::SessionLogout>();
    leptos::server_fn::axum::register_explicit::<pages::session::CurrentRole>();
    leptos::server_fn::axum::register_explicit::<pages::session::RequireRoleOrRedirect>();
    leptos::server_fn::axum::register_explicit::<pages::push::RegisterPushToken>();
    leptos::server_fn::axum::register_explicit::<pages::push::UnregisterPushToken>();
    leptos::server_fn::axum::register_explicit::<pages::push::SendBroadcast>();
    leptos::server_fn::axum::register_explicit::<pages::push::ListBroadcasts>();
    leptos::server_fn::axum::register_explicit::<pages::settings::ListSettings>();
    leptos::server_fn::axum::register_explicit::<pages::settings::UpdateSetting>();
    leptos::server_fn::axum::register_explicit::<pages::home::HomeDeliveryInfoFn>();
    leptos::server_fn::axum::register_explicit::<pages::kitchen::login::KitchenLogin>();
    leptos::server_fn::axum::register_explicit::<pages::kitchen::board::ListKitchenOrders>();
    leptos::server_fn::axum::register_explicit::<pages::kitchen::board::KitchenAdvance>();
    leptos::server_fn::axum::register_explicit::<pages::driver::login::DriverLogin>();
    leptos::server_fn::axum::register_explicit::<pages::driver::board::ListDriverOrders>();
    leptos::server_fn::axum::register_explicit::<pages::driver::board::DriverAdvance>();
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::App;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
