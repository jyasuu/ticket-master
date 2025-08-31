import { uuidv4 } from 'https://jslib.k6.io/k6-utils/1.4.0/index.js';
import http from 'k6/http';
import { check } from 'k6';

export function createEvent(baseURL, numOfAreas){
    const eventId = "event-" + uuidv4();
    const event = {
        event_name: `${eventId}`,
        artist: "k6 test",
        reservation_opening_time: "2024-01-01T10:00:00Z",
        reservation_closing_time: "2024-12-31T18:00:00Z",
        event_start_time: "2024-12-31T20:00:00Z",
        event_end_time: "2024-12-31T23:00:00Z",
        areas: []
    };

    for(let areaIdx = 0 ; areaIdx < numOfAreas ; ++areaIdx){
        event.areas.push({
            area_id: areaIdx.toString(),
            price: 100,
            row_count: 20,
            col_count: 20,
        })
    }
    
    const payload = JSON.stringify(event)

    const params = {
        headers: {
            'Content-Type': 'application/json',
        },
    };

    const url = `${baseURL}/events`
    const res = http.post(url, payload, params);
    const isValid = check(res, {
        'status is 200': (r) => r.status === 200,
    });

    if (!isValid) {
        throw new Error(`Setup failed: Expected status 200 but got ${res.status}`);
    }

    return event
}

