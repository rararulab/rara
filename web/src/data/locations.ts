/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

export const POPULAR_LOCATIONS = [
  "Remote",
  // North America
  "San Francisco, CA",
  "New York, NY",
  "Seattle, WA",
  "Austin, TX",
  "Los Angeles, CA",
  "Chicago, IL",
  "Boston, MA",
  "Denver, CO",
  "San Jose, CA",
  "Portland, OR",
  "Atlanta, GA",
  "Dallas, TX",
  "Miami, FL",
  "Washington, DC",
  "Toronto, Canada",
  "Vancouver, Canada",
  "Montreal, Canada",
  // Europe
  "London, UK",
  "Berlin, Germany",
  "Munich, Germany",
  "Amsterdam, Netherlands",
  "Paris, France",
  "Dublin, Ireland",
  "Zurich, Switzerland",
  "Stockholm, Sweden",
  "Barcelona, Spain",
  "Lisbon, Portugal",
  "Copenhagen, Denmark",
  "Helsinki, Finland",
  "Warsaw, Poland",
  "Prague, Czech Republic",
  // Asia-Pacific
  "Tokyo, Japan",
  "Singapore",
  "Sydney, Australia",
  "Melbourne, Australia",
  "Bangalore, India",
  "Shanghai, China",
  "Beijing, China",
  "Shenzhen, China",
  "Seoul, South Korea",
  "Taipei, Taiwan",
  "Hong Kong",
  // Other
  "Tel Aviv, Israel",
  "Dubai, UAE",
  "Sao Paulo, Brazil",
] as const;

export const RECENT_LOCATIONS_KEY = "job-discovery-recent-locations";
export const MAX_RECENT_LOCATIONS = 10;
