// Copyright 2026 Rararulab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

pub(super) use rara_trading::feed::market_candle::{
    DEFAULT_MARKET_CANDLE_REQUEST_BUDGET_PER_SECOND, market_candle_config_fanout_safety,
    market_candle_fanout_safety, unsafe_market_candle_fanout_message,
    validate_market_candle_request_budget,
};
