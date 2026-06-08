#![recursion_limit = "512"]

pub mod app;
pub mod branding;
pub mod components;
pub mod hydration_scripts;
pub mod i18n;
pub mod live;
pub mod order_cache;
pub mod pages;
pub mod stripe;
pub mod utils;

/// Explicitly register all `#[server]` items so they survive linking from a
/// downstream binary. Inventory-based auto-registration is unreliable when the
/// frontend is built as a cdylib+rlib and consumed by the server binary.
#[cfg(feature = "ssr")]
pub fn register_server_fns() {
    leptos::server_fn::axum::register_explicit::<branding::GetShopName>();
    leptos::server_fn::axum::register_explicit::<i18n::GetI18nEnabled>();
    leptos::server_fn::axum::register_explicit::<stripe::GetStripeMode>();
    leptos::server_fn::axum::register_explicit::<pages::menu::ListMenu>();
    leptos::server_fn::axum::register_explicit::<pages::menu::ListAdminMenu>();
    leptos::server_fn::axum::register_explicit::<pages::menu::ListMenuCategory>();
    leptos::server_fn::axum::register_explicit::<pages::lieferservice::LoadDeliveryArea>();
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
    leptos::server_fn::axum::register_explicit::<pages::locality::GetPostcodeHints>();
    leptos::server_fn::axum::register_explicit::<pages::locality::SetPostcodeHints>();
    leptos::server_fn::axum::register_explicit::<pages::locality::GetDeliveryAreas>();
    leptos::server_fn::axum::register_explicit::<pages::locality::SetDeliveryAreas>();
    leptos::server_fn::axum::register_explicit::<pages::locality::GeocodeForAdmin>();
    leptos::server_fn::axum::register_explicit::<pages::admin::localities::LoadLocalitiesAdmin>();
    leptos::server_fn::axum::register_explicit::<pages::admin::localities::SaveBypassSettings>();
    leptos::server_fn::axum::register_explicit::<pages::order::StartTour>();
    leptos::server_fn::axum::register_explicit::<pages::order::GetTour>();
    leptos::server_fn::axum::register_explicit::<pages::order::TourStopDelivered>();
    leptos::server_fn::axum::register_explicit::<pages::order::FinishTour>();
    leptos::server_fn::axum::register_explicit::<pages::order::RemoveTourStop>();
    leptos::server_fn::axum::register_explicit::<pages::order::CancelTour>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::ListAdminOrders>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::UpdateOrderStatus>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::ReprintOrder>();
    leptos::server_fn::axum::register_explicit::<pages::admin::orders::GetAdminOrder>();
    leptos::server_fn::axum::register_explicit::<pages::admin::home::LoadAdminStats>();
    leptos::server_fn::axum::register_explicit::<pages::admin::history::ListAdminHistory>();
    leptos::server_fn::axum::register_explicit::<pages::admin::customers::ListAdminCustomers>();
    leptos::server_fn::axum::register_explicit::<pages::admin::customers::GetAdminCustomer>();
    leptos::server_fn::axum::register_explicit::<pages::admin::customers::UpdateAdminCustomer>();
    leptos::server_fn::axum::register_explicit::<pages::admin::customers::SetCustomerBlacklist>();
    leptos::server_fn::axum::register_explicit::<pages::vouchers::ValidateVoucher>();
    leptos::server_fn::axum::register_explicit::<pages::admin::vouchers::ListAdminVouchers>();
    leptos::server_fn::axum::register_explicit::<pages::admin::vouchers::CreateVoucher>();
    leptos::server_fn::axum::register_explicit::<pages::admin::vouchers::UpdateVoucher>();
    leptos::server_fn::axum::register_explicit::<pages::admin::vouchers::ToggleVoucher>();
    leptos::server_fn::axum::register_explicit::<pages::admin::vouchers::DeleteVoucher>();
    leptos::server_fn::axum::register_explicit::<pages::admin::zones::ListAdminZones>();
    leptos::server_fn::axum::register_explicit::<pages::admin::zones::CreateZone>();
    leptos::server_fn::axum::register_explicit::<pages::admin::zones::UpdateZone>();
    leptos::server_fn::axum::register_explicit::<pages::admin::zones::DeleteZone>();
    leptos::server_fn::axum::register_explicit::<pages::admin::address_attempts::ListAddressAttempts>(
    );
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::GetPdfDefaults>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::ListPdfCovers>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::ActivatePdfCover>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::DeletePdfCover>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::ListPdfThemes>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::SavePdfTheme>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::ActivatePdfTheme>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pdf::DeletePdfTheme>();
    leptos::server_fn::axum::register_explicit::<pages::admin::pricing::AnalyzePricing>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::ListOptionGroupsAdmin>(
    );
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::CreateOptionGroup>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::UpdateOptionGroup>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::DeleteOptionGroup>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::CreateOption>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::UpdateOption>();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::DeleteOption>();
    leptos::server_fn::axum::register_explicit::<
        pages::admin::options_admin::ListCategoriesForAttach,
    >();
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::ListGroupAttachments>(
    );
    leptos::server_fn::axum::register_explicit::<pages::admin::options_admin::AttachGroupToCategory>(
    );
    leptos::server_fn::axum::register_explicit::<
        pages::admin::options_admin::DetachGroupFromCategory,
    >();
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
    // Opening-hours editor + quick open/close + snooze (/admin/hours).
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::LoadHoursAdmin>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::UpdateHourRow>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::UpsertSpecialHours>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::DeleteSpecialHours>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::SetOrdersOpen>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::SetOrdersSchedule>();
    leptos::server_fn::axum::register_explicit::<pages::admin::hours::SnoozeOrders>();
    // Live customer channel: admin → customer order message + delivery ack.
    leptos::server_fn::axum::register_explicit::<pages::order::SetOrderMessage>();
    leptos::server_fn::axum::register_explicit::<pages::order::AckOrderMessage>();
    // Shop open/closed status for the menu page's status bar.
    leptos::server_fn::axum::register_explicit::<pages::menu::ShopStatus>();
}

#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::App;
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
