use serde::Deserialize;
use move_core_types::identifier::Identifier;
use move_core_types::language_storage::StructTag;

mod common;
use common::assets::{modules, transactions};
use common::mock::*;
use common::addr::*;
use common::utils;

#[test]
fn publish_module() {
    new_test_ext().execute_with(|| {
        let origin = origin_ps_acc();
        utils::publish_module(origin, &modules::user::STORE, None).unwrap();
    });
}

#[test]
fn execute_script() {
    new_test_ext().execute_with(|| {
        let origin = origin_ps_acc();

        utils::publish_module(origin, &modules::user::STORE, None).unwrap();

        #[derive(Deserialize, Debug, PartialEq)]
        struct StoreU64 {
            pub val: u64,
        }

        utils::execute_tx(origin, &transactions::STORE_U64, None).unwrap();

        let tag = StructTag {
            address: origin_move_addr(),
            module: Identifier::new(modules::user::STORE.name()).unwrap(),
            name: Identifier::new("U64").unwrap(),
            type_params: vec![],
        };
        utils::check_storage_res(to_move_addr(origin), tag, StoreU64 { val: 42 });
    });
}
