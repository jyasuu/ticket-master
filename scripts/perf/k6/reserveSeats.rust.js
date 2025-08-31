import http from 'k6/http';
import { check, group } from 'k6';
import { randomIntBetween } from 'https://jslib.k6.io/k6-utils/1.4.0/index.js';
import exec from 'k6/execution';

export function reserveSeats(baseURL, event, reservationTime, reservationCounter){
  const eventId = event.event_name
  const numOfAreas = event.areas.length
  group('reserve seats', function(){
    const areaIdx = Math.floor(Math.random() * numOfAreas);
    const payload = JSON.stringify({
      user_id: "tall154215",
      event_id: `${eventId}`,
      area_id: event.areas[areaIdx].area_id,
      num_of_seats: randomIntBetween(1, 4),
      reservation_type: "RANDOM"
    })

    const postParams = {
      headers: {
        'Content-Type': 'application/json',
      },
      responseType: "text",
    };

    const reserve_seats_url = `${baseURL}/reservations`
    const reserve_seats_res = http.post(reserve_seats_url, payload, postParams)
    check(reserve_seats_res, {
      'status is 200': (r) => r.status === 200,
    });

    if(reserve_seats_res.status != 200) {
      return;
    }

    const response = JSON.parse(reserve_seats_res.body);
    const reservationId = response.data;
    const get_reservation_url = `${baseURL}/reservations/${reservationId}`
    const getParams = {
      headers: {
        'Content-Type': 'application/json',
      },
    }
    const get_reservation_res = http.get(http.url`${get_reservation_url}`, getParams)
    check(get_reservation_res, {
      'status is 200': (r) => r.status === 200,
    })

    if(get_reservation_res.status === 200){
      reservationTime.add(reserve_seats_res.timings.duration + get_reservation_res.timings.duration, [`${exec.vu.tags}`])
      reservationCounter.add(1)
    }
  })
}
