// pub const USER_ID: &str = "ae35bbb9-65da-477d-b856-7dbd87546441";
// pub const PLAN_LEETCODE_ID: &str = "018c344a-78a8-79bc-aefd-09113eaf5cb3";
// pub const PLAN_NOTION_ID: &str = "018c344b-da85-70dc-ae6f-5b919847dbbf";
// pub const PLAN_SUPABASE_ID: &str = "018c344d-5957-72cf-816b-938dea2f5c76";
// pub const PLAN_VERSION_LEETCODE_ID: &str = "018c344a-78a9-7e2b-af90-5748672711f8";
// pub const PLAN_VERSION_NOTION_ID: &str = "018c344b-da87-7392-bbae-c5c8780adb1b";
// pub const PLAN_VERSION_SUPABASE_ID: &str = "018c35cc-3f41-7551-b7b6-f8bbcd62b784";

pub mod seed {
    use uuid::{uuid, Uuid};

    pub const TENANT_ID: Uuid = uuid!("018c2c82-3df1-7e84-9e05-6e141d0e751a");
    pub const CUSTOMER_SPORTIFY_ID: Uuid = uuid!("018c345f-7324-7cd2-a692-78e5ab9158e0");
    pub const CUSTOMER_UBER_ID: Uuid = uuid!("018c345f-dff1-7857-b988-6c792ed6fa3f");
    pub const CUSTOMER_COMODO_ID: Uuid = uuid!("018c3463-05f3-7c1f-92b1-ddb1f70905a2");

    // MONTHLY - NOTION - start-date=2023-11-04 - billing-day=1;
    pub const SUBSCRIPTION_SPORTIFY_ID1: Uuid = uuid!("018c3475-bdc5-77dd-9e26-e9a7fdd60426");
    // MONTHLY - SUPABASE - start-date=2023-11-05 - billing-day=1
    pub const SUBSCRIPTION_SPORTIFY_ID2: Uuid = uuid!("018c3762-d554-7339-b13d-6fff8c9b76a0");

    // ANNUAL - NOTION - start-date=2023-11-04 - billing-day=1
    pub const SUBSCRIPTION_UBER_ID1: Uuid = uuid!("018c3477-2274-7029-9743-b3a4eb779399");
    // ANNUAL - LEETCODE - start-date=2023-11-04 - billing-day=15
    pub const SUBSCRIPTION_UBER_ID2: Uuid = uuid!("018c3479-fa9d-713f-b74f-6d9cc22cf110");

    // MONTHLY - SUPABASE - start-date=2023-11-05 - billing-day=1
    pub const SUBSCRIPTION_COMODO_ID1: Uuid = uuid!("018c3763-070e-709d-8413-f42828e71943");
    // MONTHLY - LEETCODE - start-date=2023-11-04 - billing-day=31
    pub const SUBSCRIPTION_COMODO_ID2: Uuid = uuid!("018c347a-b42b-709f-8e70-b0b63029aa35");
}
